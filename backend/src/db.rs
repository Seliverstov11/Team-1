use std::sync::Arc;

pub type Db = Arc<libsql::Database>;

/// Без переменных окружения поднимается локальный файл: проект должен
/// запускаться по README на чистой машине, без токенов и без интернета (H6, H12).
/// Turso подключается только когда заданы TURSO_DATABASE_URL и TURSO_AUTH_TOKEN.
pub async fn connect() -> anyhow::Result<Db> {
    let db = match (
        std::env::var("TURSO_DATABASE_URL").ok(),
        std::env::var("TURSO_AUTH_TOKEN").ok(),
    ) {
        (Some(url), Some(token)) if !url.is_empty() => {
            tracing::info!("БД: Turso");
            libsql::Builder::new_remote(url, token).build().await?
        }
        _ => {
            let path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "data.db".into());
            tracing::info!("БД: локальный файл {path}");
            libsql::Builder::new_local(&path).build().await?
        }
    };

    migrate(&db).await?;
    Ok(Arc::new(db))
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
