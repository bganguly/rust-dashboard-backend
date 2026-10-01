use sqlx::PgPool;
use std::fs;
use std::path::Path;

pub async fn run(pool: &PgPool, migrations_path: &str) -> Result<(), sqlx::Error> {
    if flyway_versions(pool).await > 0 {
        log::info!("migrations: Flyway has applied versions — skipping Rust migrations");
        return Ok(());
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS rust_schema_migrations (
            version    INTEGER PRIMARY KEY,
            filename   TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;

    let mut migrations = collect_migrations(migrations_path);
    migrations.sort_by_key(|(v, _)| *v);

    for (version, path) in &migrations {
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM rust_schema_migrations WHERE version = $1)",
        )
        .bind(*version)
        .fetch_one(pool)
        .await?;

        if applied {
            continue;
        }

        let sql = fs::read_to_string(path)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e.to_string())))?;

        sqlx::query(&sql).execute(pool).await?;

        let filename = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path.as_str())
            .to_string();

        sqlx::query(
            "INSERT INTO rust_schema_migrations (version, filename) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(*version)
        .bind(&filename)
        .execute(pool)
        .await?;

        log::info!("migrations: applied V{version} ({filename})");
    }
    Ok(())
}

async fn flyway_versions(pool: &PgPool) -> i64 {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'flyway_schema_history'
        )",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    if !exists {
        return 0;
    }

    sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

fn collect_migrations(dir: &str) -> Vec<(i32, String)> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("migrations: cannot read dir {dir}: {e}");
            return vec![];
        }
    };

    let re = regex_version();
    let mut out = vec![];
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(v) = re(&name) {
            out.push((v, entry.path().to_string_lossy().to_string()));
        }
    }
    out
}

fn regex_version() -> impl Fn(&str) -> Option<i32> {
    |name: &str| {
        if !name.starts_with('V') {
            return None;
        }
        let rest = &name[1..];
        let end = rest.find("__")?;
        rest[..end].parse::<i32>().ok()
    }
}
