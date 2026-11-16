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

    Ok(())
}
