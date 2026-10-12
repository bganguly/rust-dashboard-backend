use crate::cache::AggregatesCache;
use crate::model::*;
use crate::services::query::QueryArgs;
use crate::services::orders_query::{
    build_count_cache_key, build_order_by, build_order_where,
    flip_order_by, has_short_token, safe_order_dir, safe_order_sort,
};
use crate::services::orders_store::{
    build_result, collect_order_rows, fetch_items,
    read_count_cache, write_count_cache, ISO_FMT,
};
use chrono::NaiveDateTime;
use sqlx::{PgPool, Row};

pub use crate::services::orders_query::split_trim;

pub const COUNT_CAP: i64 = 10_000;
pub const COUNT_SENTINEL: i64 = 10_001;
pub fn is_approximate(n: i64) -> bool { n == COUNT_SENTINEL }
pub fn adjust_count(n: i64) -> i64 { if n == COUNT_SENTINEL { COUNT_CAP } else { n } }

pub async fn list_orders(
    pool: &PgPool,
    q: &str, page: i32, page_size: i32,
    sort: &str, dir: &str,
    status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<OrderListResult, sqlx::Error> {
    let page_size = page_size.max(1).min(100);
    let page = page.max(1);

    let safe_sort = safe_order_sort(sort);
    let safe_dir = safe_order_dir(dir);

    let mut qa = QueryArgs::new();
    let (where_clause, needs_region_join) =
        build_order_where(q, status, region_code, from, to, min_total, max_total, &mut qa);

    let raw_total = exact_count_internal(pool, q, status, region_code, from, to, min_total, max_total).await?;
    let approximate = is_approximate(raw_total);
    let total = adjust_count(raw_total);
    let total_pages = ((total as f64) / (page_size as f64)).ceil() as i32;

    let order_by = build_order_by(safe_sort, safe_dir);
    let use_reverse = total_pages > 1 && page == total_pages;

    let (limit, offset, effective_order_by) = if use_reverse {
        let lim = total as i32 - (total_pages - 1) * page_size;
        let lim = lim.max(1);
        (lim, 0i32, flip_order_by(&order_by))
    } else {
        (page_size, (page - 1) * page_size, order_by.clone())
    };

    let region_join = if needs_region_join {
        r#"JOIN regions r ON r.id = o."regionId""#
    } else {
        r#"JOIN regions r ON r.id = o."regionId""#
    };

    let lp = qa.add_i32(limit);
    let op = qa.add_i32(offset);

    let sql = format!(
        r#"SELECT o.id, o.status::text, CAST(o.total AS FLOAT8), o.currency, o.notes, o."placedAt",
                  c.id AS c_id, c.email, c."firstName", c."lastName", c.phone,
                  r.id AS r_id, r.code AS r_code, r.name AS r_name
           FROM orders o
           JOIN customers c ON c.id = o."customerId"
           {region_join}
           {where_clause}
           ORDER BY {effective_order_by}
           LIMIT {lp} OFFSET {op}"#
    );

    let rows = sqlx::query_with(&sql, qa.into_args())
        .fetch_all(pool)
        .await?;

    let mut order_rows = collect_order_rows(&rows)?;
    if use_reverse {
        order_rows.reverse();
    }

    build_result(pool, order_rows, page, page_size, total, total_pages, approximate).await
}

pub async fn list_orders_by_cursor(
    pool: &PgPool,
    q: &str, page: i32, page_size: i32,
    status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
    cursor_id: i32, cursor_placed_at: &str, forward: bool,
) -> Result<OrderListResult, sqlx::Error> {
    let page_size = page_size.max(1).min(100);

    let mut qa = QueryArgs::new();
    let (where_clause, _) =
        build_order_where(q, status, region_code, from, to, min_total, max_total, &mut qa);

    let raw_total = exact_count_internal(pool, q, status, region_code, from, to, min_total, max_total).await?;
    let approximate = is_approximate(raw_total);
    let total = adjust_count(raw_total);
    let total_pages = ((total as f64) / (page_size as f64)).ceil() as i32;

    let cp = qa.add_str(cursor_placed_at.to_string());
    let ci = qa.add_i32(cursor_id);
    let cursor_clause = if forward {
        format!(r#"(o."placedAt", o.id) < ({cp}::timestamp, {ci})"#)
    } else {
        format!(r#"(o."placedAt", o.id) > ({cp}::timestamp, {ci})"#)
    };

    let combined_where = if where_clause.is_empty() {
        format!("WHERE {cursor_clause}")
    } else {
        format!("{where_clause} AND {cursor_clause}")
    };

    let order_by = if forward {
        r#"o."placedAt" DESC, o.id DESC"#
    } else {
        r#"o."placedAt" ASC, o.id ASC"#
    };

    let lp = qa.add_i32(page_size);

    let sql = format!(
        r#"SELECT o.id, o.status::text, CAST(o.total AS FLOAT8), o.currency, o.notes, o."placedAt",
                  c.id AS c_id, c.email, c."firstName", c."lastName", c.phone,
                  r.id AS r_id, r.code AS r_code, r.name AS r_name
           FROM orders o
           JOIN customers c ON c.id = o."customerId"
           JOIN regions r ON r.id = o."regionId"
           {combined_where}
           ORDER BY {order_by}
           LIMIT {lp}"#
    );

    let rows = sqlx::query_with(&sql, qa.into_args())
        .fetch_all(pool)
        .await?;

    let mut order_rows = collect_order_rows(&rows)?;
    if !forward {
        order_rows.reverse();
    }

    build_result(pool, order_rows, page, page_size, total, total_pages, approximate).await
}

async fn exact_count_internal(
    pool: &PgPool,
    q: &str, status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<i64, sqlx::Error> {
    let cache_key = build_count_cache_key(q, status, region_code, from, to, min_total, max_total);
    if let Some(n) = read_count_cache(pool, &cache_key).await {
        return Ok(n);
    }

    let mut qa = QueryArgs::new();
    let (where_clause, needs_region_join) =
        build_order_where(q, status, region_code, from, to, min_total, max_total, &mut qa);
    let region_join = if needs_region_join {
        r#"JOIN regions r ON r.id = o."regionId" "#
    } else {
        ""
    };

    let n = if has_short_token(q) {
        let cap = qa.add_i64(COUNT_SENTINEL);
        let sql = format!(
            "SELECT COUNT(*) FROM (SELECT 1 FROM orders o {region_join}{where_clause} LIMIT {cap}) _cap"
        );
        let n: i64 = sqlx::query_scalar_with(&sql, qa.into_args())
            .fetch_one(pool)
            .await?;
        if n < COUNT_SENTINEL {
            write_count_cache(pool, &cache_key, n).await;
        }
        n
    } else {
        let sql = format!("SELECT COUNT(*) FROM orders o {region_join}{where_clause}");
        let n: i64 = sqlx::query_scalar_with(&sql, qa.into_args())
            .fetch_one(pool)
            .await?;
        write_count_cache(pool, &cache_key, n).await;
        n
    };

    Ok(n)
}

pub async fn exact_count_uncapped(
    pool: &PgPool,
    q: &str, status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<i64, sqlx::Error> {
    let cache_key = build_count_cache_key(q, status, region_code, from, to, min_total, max_total);
    let t0 = std::time::Instant::now();
    if let Some(n) = read_count_cache(pool, &cache_key).await {
        log::info!("[COUNT] count_cache HIT key={} in {}ms", cache_key, t0.elapsed().as_millis());
        return Ok(n);
    }
    log::info!("[COUNT] count_cache MISS key={} lookup={}ms", cache_key, t0.elapsed().as_millis());

    let mut qa = QueryArgs::new();
    let (where_clause, needs_region_join) =
        build_order_where(q, status, region_code, from, to, min_total, max_total, &mut qa);
    let region_join = if needs_region_join {
        r#"JOIN regions r ON r.id = o."regionId" "#
    } else {
        ""
    };

    let sql = format!("SELECT COUNT(*) FROM orders o {region_join}{where_clause}");
    let t1 = std::time::Instant::now();
    let n: i64 = sqlx::query_scalar_with(&sql, qa.into_args())
        .fetch_one(pool)
        .await?;
    log::info!("[COUNT] COUNT(*) done in {}ms result={}", t1.elapsed().as_millis(), n);
    write_count_cache(pool, &cache_key, n).await;
    Ok(n)
}

pub struct CreateResult {
    pub id: i32,
    pub status: String,
    pub total: f64,
    pub placed_at: String,
}

pub async fn create_order(pool: &PgPool, req: CreateOrderRequest) -> Result<CreateResult, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let customer_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM customers WHERE id = $1)")
            .bind(req.customer_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    if !customer_exists {
        return Err(format!("customer not found: {}", req.customer_id));
    }

    let region_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM regions WHERE id = $1)")
            .bind(req.region_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    if !region_exists {
        return Err(format!("region not found: {}", req.region_id));
    }

    if req.items.is_empty() {
        return Err("items must not be empty".to_string());
    }

    let currency = if req.currency.is_empty() {
        "USD".to_string()
    } else {
        req.currency.clone()
    };
    let notes: Option<String> = if req.notes.is_empty() { None } else { Some(req.notes.clone()) };

    let row = sqlx::query(
        r#"INSERT INTO orders ("customerId","regionId",currency,notes,status,total,"placedAt","updatedAt")
           VALUES ($1,$2,$3,$4,'PENDING',0,NOW(),NOW()) RETURNING id,"placedAt""#,
    )
    .bind(req.customer_id)
    .bind(req.region_id)
    .bind(&currency)
    .bind(&notes)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let order_id: i32 = row.try_get("id").map_err(|e| e.to_string())?;
    let placed_at: NaiveDateTime = row.try_get("placedAt").map_err(|e| e.to_string())?;

    let mut grand_total: f64 = 0.0;
    for item in &req.items {
        let product_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE id = $1)")
                .bind(item.product_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if !product_exists {
            return Err(format!("product not found: {}", item.product_id));
        }
        let line = item.unit_price * (item.quantity as f64) * (1.0 - item.discount);
        grand_total += line;
        sqlx::query(
            r#"INSERT INTO order_items ("orderId","productId",quantity,"unitPrice",discount)
               VALUES ($1,$2,$3,$4,$5)"#,
        )
        .bind(order_id)
        .bind(item.product_id)
        .bind(item.quantity)
        .bind(item.unit_price)
        .bind(item.discount)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    sqlx::query(r#"UPDATE orders SET total=$1,"updatedAt"=NOW() WHERE id=$2"#)
        .bind(grand_total)
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let date_str = placed_at.format("%Y-%m-%d").to_string();
    sqlx::query(
        r#"INSERT INTO daily_order_count (date,"totalOrders") VALUES ($1::date,1)
           ON CONFLICT (date) DO UPDATE SET "totalOrders"=daily_order_count."totalOrders"+1"#,
    )
    .bind(&date_str)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(CreateResult {
        id: order_id,
        status: "PENDING".to_string(),
        total: grand_total,
        placed_at: placed_at.format(ISO_FMT).to_string(),
    })
}

pub async fn invalidate_after_create(pool: &PgPool, cache: &AggregatesCache, order_id: i32) {
    let search_text: Option<String> =
        sqlx::query_scalar("SELECT search_text FROM orders WHERE id = $1")
            .bind(order_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let _ = sqlx::query(
        r#"DELETE FROM count_cache WHERE
           substring(cache_key from 'q=([^&]*)') = ''
           OR ($1::text IS NOT NULL AND $1::text ILIKE '%' || substring(cache_key from 'q=([^&]*)') || '%')"#,
    )
    .bind(search_text)
    .execute(pool)
    .await;

    cache.invalidate_all();
}

