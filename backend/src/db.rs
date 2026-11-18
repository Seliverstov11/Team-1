use std::{sync::Arc, time::Duration};

/// Обёртка над базой: каждое соединение ждёт до 5 секунд, если файл занят
/// другой записью, вместо мгновенной ошибки «database is locked».
/// Рядом — счётчик неудачных входов: он живёт столько же, сколько сервер,
/// и у каждой тестовой базы свой.
pub struct Database {
    inner: libsql::Database,
    pub logins: crate::auth::LoginLimiter,
}

impl Database {
    fn new(inner: libsql::Database) -> Self {
        Self { inner, logins: Default::default() }
    }

    pub fn connect(&self) -> libsql::Result<libsql::Connection> {
        let conn = self.inner.connect()?;
        // Удалённая Turso сама разруливает конкурентность, там настройки нет.
        let _ = conn.busy_timeout(Duration::from_secs(5));
        Ok(conn)
    }
}

pub type Db = Arc<Database>;

/// Без переменных окружения поднимается локальный файл: проект должен
/// запускаться по README на чистой машине, без токенов и без интернета (H6, H12).
/// Turso подключается только когда заданы TURSO_DATABASE_URL и TURSO_AUTH_TOKEN.
pub async fn connect() -> anyhow::Result<Db> {
    let path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "data.db".into());
    connect_with(&path).await
}

/// Локальный файл по пути — для тестов, каждому своя временная БД.
pub async fn connect_local(path: &str) -> anyhow::Result<Db> {
    let db = libsql::Builder::new_local(path).build().await?;
    enable_wal(&db).await?;
    migrate(&db).await?;
    Ok(Arc::new(Database::new(db)))
}

/// В режиме WAL чтение не блокирует запись: CRM и сайт работают с базой
/// одновременно. Настройка хранится в самом файле.
async fn enable_wal(db: &libsql::Database) -> anyhow::Result<()> {
    let conn = db.connect()?;
    let mut rows = conn.query("PRAGMA journal_mode = WAL", ()).await?;
    while rows.next().await?.is_some() {}
    Ok(())
}

async fn connect_with(path: &str) -> anyhow::Result<Db> {
    let db = match (
        std::env::var("TURSO_DATABASE_URL").ok(),
        std::env::var("TURSO_AUTH_TOKEN").ok(),
    ) {
        (Some(url), Some(token)) if !url.is_empty() => {
            tracing::info!("БД: Turso");
            libsql::Builder::new_remote(url, token).build().await?
        }
        _ => {
            tracing::info!("БД: локальный файл {path}");
            let db = libsql::Builder::new_local(path).build().await?;
            enable_wal(&db).await?;
            db
        }
    };

    migrate(&db).await?;
    Ok(Arc::new(Database::new(db)))
}

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../migrations/001_initial.sql")),
    ("002_seed_demo", include_str!("../migrations/002_seed_demo.sql")),
    (
        "003_attributes_and_limits",
        include_str!("../migrations/003_attributes_and_limits.sql"),
    ),
    (
        "004_customer_data_model",
        include_str!("../migrations/004_customer_data_model.sql"),
    ),
    (
        "005_catalog_from_customer",
        include_str!("../migrations/005_catalog_from_customer.sql"),
    ),
    ("006_crm_auth", include_str!("../migrations/006_crm_auth.sql")),
    ("007_seed_sample", include_str!("../migrations/007_seed_sample.sql")),
    ("008_search_text", include_str!("../migrations/008_search_text.sql")),
    ("010_sync_and_restock", include_str!("../migrations/010_sync_and_restock.sql")),
];

async fn migrate(db: &libsql::Database) -> anyhow::Result<()> {
    let conn = db.connect()?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name       TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
         )",
        (),
    )
    .await?;

    for (name, sql) in MIGRATIONS {
        let mut rows = conn
            .query(
                "SELECT 1 FROM schema_migrations WHERE name = ?1",
                libsql::params![*name],
            )
            .await?;
        if rows.next().await?.is_some() {
            continue;
        }

        conn.execute_batch(sql).await?;
        conn.execute(
            "INSERT INTO schema_migrations (name) VALUES (?1)",
            libsql::params![*name],
        )
        .await?;
        tracing::info!("миграция применена: {name}");
    }

    // Строки поиска для товаров, у которых их ещё нет: после миграции 008
    // и после перезаливки демо-данных.
    let n = refresh_search(&conn, "p.search_text IS NULL", libsql::Value::Null).await?;
    if n > 0 {
        tracing::info!("строки поиска собраны для товаров: {n}");
    }
    Ok(())
}

/// Нижний регистр по Unicode, «ё» → «е», всё кроме букв и цифр — пробел.
pub fn normalize_search(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.to_lowercase().chars() {
        let ch = if ch == 'ё' { 'е' } else { ch };
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// Пересобирает строку поиска у товаров, подходящих под условие
/// (`p.id = ?1`, `p.category_id = ?1` или `p.search_text IS NULL`).
pub async fn refresh_search(conn: &libsql::Connection, condition: &str, param: libsql::Value) -> anyhow::Result<usize> {
    let sql = format!(
        "SELECT p.id, p.name, IFNULL(p.variant_name, ''), IFNULL(p.sku, ''), IFNULL(c.name, '')
         FROM products p LEFT JOIN categories c ON c.id = p.category_id WHERE {condition}"
    );
    let params: Vec<libsql::Value> = if condition.contains("?1") { vec![param] } else { vec![] };
    let mut rows = conn.query(&sql, params).await?;
    let mut updates = Vec::new();
    while let Some(r) = rows.next().await? {
        let id: String = r.get(0)?;
        let text = normalize_search(&format!(
            "{} {} {} {}",
            r.get::<String>(1)?,
            r.get::<String>(2)?,
            r.get::<String>(3)?,
            r.get::<String>(4)?
        ));
        updates.push((id, text));
    }
    drop(rows);
    for (id, text) in &updates {
        conn.execute(
            "UPDATE products SET search_text = ?1 WHERE id = ?2",
            libsql::params![text.clone(), id.clone()],
        )
        .await?;
    }
    Ok(updates.len())
}

/// Слова запроса для поиска: нормализованы, у длинных отрезано окончание,
/// чтобы «коробка» находила «коробки», а «торта» — «тортов».
pub fn search_terms(q: &str) -> Vec<String> {
    normalize_search(q)
        .split_whitespace()
        .map(|w| {
            let chars: Vec<char> = w.chars().collect();
            let cut = match chars.len() {
                n if n >= 6 && chars.iter().all(|c| c.is_alphabetic()) => 2,
                5 if chars.iter().all(|c| c.is_alphabetic()) => 1,
                _ => 0,
            };
            chars[..chars.len() - cut].iter().collect()
        })
        .take(6)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{normalize_search, search_terms};

    #[test]
    fn нормализация_для_поиска() {
        assert_eq!(normalize_search("Шоколад тёмный 54%, CHO-D54"), "шоколад темный 54 cho d54");
    }

    #[test]
    fn слова_запроса() {
        assert_eq!(search_terms("Коробка торт"), vec!["короб".to_string(), "торт".into()]);
        assert_eq!(search_terms("шоколад"), vec!["шокол".to_string()]);
        assert_eq!(search_terms("CHO-D54-500"), vec!["cho".to_string(), "d54".into(), "500".into()]);
    }
}
