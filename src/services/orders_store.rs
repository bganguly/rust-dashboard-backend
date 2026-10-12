use crate::model::*;
use chrono::NaiveDateTime;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

pub(crate) const ISO_FMT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

pub(crate) struct OrderRow {
    pub id: i32,
    pub status: String,
    pub total: f64,
    pub currency: String,
    pub notes: Option<String>,
    pub placed_at: NaiveDateTime,
    pub c_id: i32,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub phone: Option<String>,
    pub r_id: i32,
    pub r_code: String,
    pub r_name: String,
}

pub(crate) fn collect_order_rows(rows: &[sqlx::postgres::PgRow]) -> Result<Vec<OrderRow>, sqlx::Error> {
    rows.iter()
        .map(|r| {
            Ok(OrderRow {
                id: r.try_get("id")?,
                status: r.try_get("status")?,
                total: r.try_get("total")?,
                currency: r.try_get("currency")?,
                notes: r.try_get("notes")?,
                placed_at: r.try_get("placedAt")?,
                c_id: r.try_get("c_id")?,
                email: r.try_get("email")?,
                first_name: r.try_get("firstName")?,
                last_name: r.try_get("lastName")?,
                phone: r.try_get("phone")?,
                r_id: r.try_get("r_id")?,
                r_code: r.try_get("r_code")?,
                r_name: r.try_get("r_name")?,
            })
        })
        .collect()
}

pub(crate) async fn fetch_items(pool: &PgPool, ids: &[i32]) -> Result<HashMap<i32, Vec<OrderItemDTO>>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        r#"SELECT oi."orderId", oi.id, oi."productId", oi.quantity,
                  CAST(oi."unitPrice" AS FLOAT8) AS "unitPrice",
                  CAST(oi.discount AS FLOAT8) AS discount,
                  pr.sku, pr.name AS p_name
           FROM order_items oi
           JOIN products pr ON pr.id = oi."productId"
           WHERE oi."orderId" = ANY($1::int4[])"#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<i32, Vec<OrderItemDTO>> = HashMap::new();
    for r in &rows {
        let order_id: i32 = r.try_get("orderId")?;
        let product_id: i32 = r.try_get("productId")?;
        map.entry(order_id).or_default().push(OrderItemDTO {
            id: r.try_get("id")?,
            product_id,
            quantity: r.try_get("quantity")?,
            unit_price: r.try_get("unitPrice")?,
            discount: r.try_get("discount")?,
            product: ProductSummaryDTO {
                id: product_id,
                sku: r.try_get("sku")?,
                name: r.try_get("p_name")?,
            },
        });
    }
    Ok(map)
}

pub(crate) async fn build_result(
    pool: &PgPool,
    rows: Vec<OrderRow>,
    page: i32, page_size: i32,
    total: i64, total_pages: i32, approximate: bool,
) -> Result<OrderListResult, sqlx::Error> {
    if rows.is_empty() {
        return Ok(OrderListResult {
            data: vec![],
            page,
            page_size,
            total,
            total_pages,
            approximate,
        });
    }

    let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
    let items_map = fetch_items(pool, &ids).await?;

    let data = rows
        .iter()
        .map(|r| OrderDTO {
            id: r.id,
            status: r.status.clone(),
            total: r.total,
            currency: r.currency.clone(),
            notes: r.notes.clone(),
            placed_at: r.placed_at.format(ISO_FMT).to_string(),
            customer: CustomerSummaryDTO {
                id: r.c_id,
                email: r.email.clone(),
                first_name: r.first_name.clone(),
                last_name: r.last_name.clone(),
            },
            region: RegionDTO {
                id: r.r_id,
                code: r.r_code.clone(),
                name: r.r_name.clone(),
            },
            items: items_map.get(&r.id).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(OrderListResult {
        data,
        page,
        page_size,
        total,
        total_pages,
        approximate,
    })
}

pub(crate) async fn read_count_cache(pool: &PgPool, key: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT total FROM count_cache WHERE cache_key=$1 AND cached_at > NOW() - INTERVAL '30 days'",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub(crate) async fn write_count_cache(pool: &PgPool, key: &str, total: i64) {
    let _ = sqlx::query(
        "INSERT INTO count_cache (cache_key,total,cached_at) VALUES ($1,$2,NOW())
         ON CONFLICT (cache_key) DO UPDATE SET total=$2, cached_at=NOW()",
    )
    .bind(key)
    .bind(total)
    .execute(pool)
    .await;
}
