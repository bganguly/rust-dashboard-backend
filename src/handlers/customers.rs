use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

#[derive(Deserialize)]
pub struct CustomerQuery {
    cursor: Option<i32>,
    limit: Option<i32>,
    q: Option<String>,
    #[serde(rename = "regionId")]
    region_id: Option<i32>,
}

pub async fn list(
    pool: web::Data<PgPool>,
    query: web::Query<CustomerQuery>,
) -> HttpResponse {
    let limit = query.limit.unwrap_or(20);
    let q = query.q.as_deref().unwrap_or("");

    match crate::services::customers::list_customers(
        &pool,
        query.cursor,
        limit,
        q,
        query.region_id,
    )
    .await
    {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
