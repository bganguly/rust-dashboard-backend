use actix_web::{web, HttpResponse};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;

#[derive(Serialize)]
pub struct DatasetBounds {
    pub from: String,
    pub to: String,
}

pub async fn get(pool: web::Data<PgPool>) -> HttpResponse {
    let row: Result<(Option<chrono::NaiveDate>, Option<chrono::NaiveDate>), sqlx::Error> =
        sqlx::query_as(r#"SELECT MIN("placedAt")::date, MAX("placedAt")::date FROM orders"#)
            .fetch_one(pool.get_ref())
            .await;

    match row {
        Ok((from, to)) => HttpResponse::Ok().json(DatasetBounds {
            from: from.map(|d| d.to_string()).unwrap_or_default(),
            to: to.map(|d| d.to_string()).unwrap_or_default(),
        }),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
