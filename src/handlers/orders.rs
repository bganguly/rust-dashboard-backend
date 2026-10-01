use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use crate::cache::AggregatesCache;
use crate::model::CreateOrderRequest;
use crate::services::orders::{
    create_order, exact_count_uncapped, invalidate_after_create,
    list_orders, list_orders_by_cursor,
};

#[derive(Deserialize, Default)]
pub struct OrderQuery {
    q: Option<String>,
    page: Option<i32>,
    #[serde(rename = "pageSize")]
    page_size: Option<i32>,
    sort: Option<String>,
    dir: Option<String>,
    status: Option<String>,
    #[serde(rename = "regionCode")]
    region_code: Option<String>,
    from: Option<String>,
    to: Option<String>,
    #[serde(rename = "minTotal")]
    min_total: Option<f64>,
    #[serde(rename = "maxTotal")]
    max_total: Option<f64>,
    #[serde(rename = "cursorId")]
    cursor_id: Option<String>,
    #[serde(rename = "cursorPlacedAt")]
    cursor_placed_at: Option<String>,
    #[serde(rename = "cursorDir")]
    cursor_dir: Option<String>,
}

pub async fn list(
    pool: web::Data<PgPool>,
    query: web::Query<OrderQuery>,
) -> HttpResponse {
    let q = query.q.as_deref().unwrap_or("");
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20);
    let sort = query.sort.as_deref().unwrap_or("placedAt");
    let dir = query.dir.as_deref().unwrap_or("desc");
    let status = query.status.as_deref().unwrap_or("");
    let region_code = query.region_code.as_deref().unwrap_or("");
    let from = query.from.as_deref().unwrap_or("");
    let to = query.to.as_deref().unwrap_or("");

    let use_cursor = query.cursor_id.is_some()
        && query.cursor_placed_at.is_some()
        && (sort == "placedAt")
        && (dir == "desc" || dir.is_empty());

    if use_cursor {
        let cursor_id_str = query.cursor_id.as_deref().unwrap_or("");
        let cursor_id = match cursor_id_str.parse::<i32>() {
            Ok(n) => n,
            Err(_) => {
                return HttpResponse::BadRequest().json(json!({"error": "invalid cursorId"}));
            }
        };
        let cursor_placed_at = query.cursor_placed_at.as_deref().unwrap_or("");
        let forward = query.cursor_dir.as_deref().unwrap_or("next") != "prev";

        return match list_orders_by_cursor(
            &pool, q, page, page_size, status, region_code,
            from, to, query.min_total, query.max_total,
            cursor_id, cursor_placed_at, forward,
        )
        .await
        {
            Ok(r) => HttpResponse::Ok().json(r),
            Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
        };
    }

    match list_orders(
        &pool, q, page, page_size, sort, dir,
        status, region_code, from, to,
        query.min_total, query.max_total,
    )
    .await
    {
        Ok(r) => HttpResponse::Ok().json(r),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[derive(Deserialize, Default)]
pub struct CountQuery {
    q: Option<String>,
    status: Option<String>,
    #[serde(rename = "regionCode")]
    region_code: Option<String>,
    from: Option<String>,
    to: Option<String>,
    #[serde(rename = "minTotal")]
    min_total: Option<f64>,
    #[serde(rename = "maxTotal")]
    max_total: Option<f64>,
}

pub async fn count(
    pool: web::Data<PgPool>,
    query: web::Query<CountQuery>,
) -> HttpResponse {
    let q = query.q.as_deref().unwrap_or("");
    let status = query.status.as_deref().unwrap_or("");
    let region_code = query.region_code.as_deref().unwrap_or("");
    let from = query.from.as_deref().unwrap_or("");
    let to = query.to.as_deref().unwrap_or("");

    match exact_count_uncapped(&pool, q, status, region_code, from, to, query.min_total, query.max_total).await {
        Ok(n) => HttpResponse::Ok().json(json!({"total": n})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

pub async fn create(
    pool: web::Data<PgPool>,
    cache: web::Data<AggregatesCache>,
    body: web::Json<CreateOrderRequest>,
) -> HttpResponse {
    match create_order(&pool, body.into_inner()).await {
        Ok(result) => {
            let pool_clone = (**pool).clone();
            let cache_data = cache.clone();
            let order_id = result.id;
            tokio::spawn(async move {
                invalidate_after_create(&pool_clone, &cache_data, order_id).await;
            });
            HttpResponse::Created().json(json!({
                "id": result.id,
                "status": result.status,
                "total": result.total,
                "placedAt": result.placed_at,
            }))
        }
        Err(e) => HttpResponse::BadRequest().json(json!({"error": e})),
    }
}
