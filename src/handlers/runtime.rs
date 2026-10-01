use actix_web::{web, HttpResponse};
use serde_json::json;
use sqlx::PgPool;
use std::env;

pub async fn runtime() -> HttpResponse {
    let rt = env::var("BACKEND_RUNTIME").unwrap_or_else(|_| "rust".to_string());
    HttpResponse::Ok().json(json!({"runtime": rt}))
}

pub async fn status() -> HttpResponse {
    let rt = env::var("BACKEND_RUNTIME").unwrap_or_else(|_| "rust".to_string());
    HttpResponse::Ok().json(json!({
        "runtime": rt,
        "typesense": false,
        "searchMode": "postgres-ilike"
    }))
}

pub async fn seed_stats(pool: web::Data<PgPool>) -> HttpResponse {
    match crate::services::stats::get_seed_stats(&pool).await {
        Ok((customers, products)) => HttpResponse::Ok().json(json!({
            "customerCount": customers,
            "productCount": products
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}
