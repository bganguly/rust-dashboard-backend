use actix_web::{web, HttpResponse};
use serde_json::json;
use sqlx::PgPool;

pub async fn list(pool: web::Data<PgPool>) -> HttpResponse {
    match crate::services::regions::list_regions(&pool).await {
        Ok(regions) => HttpResponse::Ok().json(regions),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
