use sqlx::postgres::{PgPool, PgPoolOptions};
use std::env;

pub async fn create_pool() -> Result<PgPool, sqlx::Error> {
    let dsn = if let Ok(url) = env::var("DATABASE_URL") {
        url
    } else {
        let host = env::var("DB_HOST").unwrap_or_else(|_| "localhost".to_string());
        let port = env::var("DB_PORT").unwrap_or_else(|_| "5432".to_string());
        let user = env::var("DB_USER").unwrap_or_default();
        let password = env::var("DB_PASSWORD").unwrap_or_default();
        let name = env::var("DB_NAME").unwrap_or_default();
        let sslmode = env::var("DB_SSLMODE").unwrap_or_else(|_| "disable".to_string());
        format!(
            "host={host} port={port} user={user} password={password} dbname={name} sslmode={sslmode}"
        )
    };
    PgPoolOptions::new().max_connections(20).connect(&dsn).await
}
