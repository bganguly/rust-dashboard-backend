use sqlx::PgPool;

pub async fn get_seed_stats(pool: &PgPool) -> Result<(i64, i64), sqlx::Error> {
    let customers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
        .fetch_one(pool)
        .await?;
    let products: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products")
        .fetch_one(pool)
        .await?;
    Ok((customers, products))
}
