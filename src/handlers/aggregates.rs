use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use crate::cache::{cache_key, AggregatesCache};
use crate::services::aggregates::{get_daily_aggregates, get_exact_total};
use crate::services::orders::{adjust_count, is_approximate};

#[derive(Deserialize)]
pub struct AggQuery {
    from: Option<String>,
    to: Option<String>,
    q: Option<String>,
    status: Option<String>,
    #[serde(rename = "regionCode")]
    region_code: Option<String>,
    #[serde(rename = "minTotal")]
    min_total: Option<f64>,
    #[serde(rename = "maxTotal")]
    max_total: Option<f64>,
    #[serde(rename = "topCategories")]
    top_categories: Option<i32>,
    #[serde(rename = "includeData", default = "default_true")]
    include_data: bool,
    #[serde(rename = "includeTotal", default = "default_true")]
    include_total: bool,
}

fn default_true() -> bool { true }

pub async fn get(
    pool: web::Data<PgPool>,
    cache: web::Data<AggregatesCache>,
    query: web::Query<AggQuery>,
) -> HttpResponse {
    let from = match &query.from {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return HttpResponse::BadRequest().json(json!({"error": "from and to are required"})),
    };
    let to = match &query.to {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return HttpResponse::BadRequest().json(json!({"error": "from and to are required"})),
    };

    let q = query.q.as_deref().unwrap_or("");
    let status = query.status.as_deref().unwrap_or("");
    let region_code = query.region_code.as_deref().unwrap_or("");
    let top_categories = query.top_categories.unwrap_or(5);
    let include_data = query.include_data;
    let include_total = query.include_total;

    let no_filters = q.is_empty() && status.is_empty() && region_code.is_empty()
        && query.min_total.is_none() && query.max_total.is_none();

    let ck = if no_filters && include_data && include_total {
        let k = cache_key(&from, &to, top_categories);
        if let Some(cached) = cache.get(&k) {
            return HttpResponse::Ok().json(cached);
        }
        Some(k)
    } else {
        None
    };

    let pool_d = pool.clone();
    let pool_t = pool.clone();
    let from_d = from.clone();
    let to_d = to.clone();
    let from_t = from.clone();
    let to_t = to.clone();
    let q_d = q.to_string();
    let q_t = q.to_string();
    let status_d = status.to_string();
    let status_t = status.to_string();
    let rc_d = region_code.to_string();
    let rc_t = region_code.to_string();
    let min_d = query.min_total;
    let max_d = query.max_total;
    let min_t = query.min_total;
    let max_t = query.max_total;

    let data_fut = async move {
        if include_data {
            Some(
                get_daily_aggregates(
                    &pool_d, &from_d, &to_d, &q_d, &status_d, &rc_d,
                    min_d, max_d, top_categories,
                )
                .await,
            )
        } else {
            None
        }
    };

    let total_fut = async move {
        if include_total {
            Some(
                get_exact_total(&pool_t, &from_t, &to_t, &q_t, &status_t, &rc_t, min_t, max_t)
                    .await,
            )
        } else {
            None
        }
    };

    let (data_res, total_res) = tokio::join!(data_fut, total_fut);

    let mut body = serde_json::Map::new();

    if let Some(res) = data_res {
        match res {
            Ok(data) => body.insert("data".to_string(), serde_json::to_value(data).unwrap()),
            Err(e) => return HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
        };
    }

    if let Some(res) = total_res {
        match res {
            Ok(raw) => {
                body.insert("totalOrders".to_string(), json!(adjust_count(raw)));
                body.insert("totalOrdersApproximate".to_string(), json!(is_approximate(raw)));
            }
            Err(e) => return HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
        }
    }

    let value = serde_json::Value::Object(body);
    if let Some(k) = ck {
        cache.put(k, value.clone());
    }
    HttpResponse::Ok().json(value)
}
