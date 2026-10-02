use crate::cache::AggregatesCache;
use crate::model::*;
use crate::services::query::QueryArgs;
use chrono::NaiveDateTime;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

pub const COUNT_CAP: i64 = 10_000;
pub const COUNT_SENTINEL: i64 = 10_001;
pub fn is_approximate(n: i64) -> bool { n == COUNT_SENTINEL }
pub fn adjust_count(n: i64) -> i64 { if n == COUNT_SENTINEL { COUNT_CAP } else { n } }

const ISO_FMT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

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

    let sql = format!("SELECT COUNT(*) FROM orders o {region_join}{where_clause}");
    let n: i64 = sqlx::query_scalar_with(&sql, qa.into_args())
        .fetch_one(pool)
        .await?;
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

struct OrderRow {
    id: i32,
    status: String,
    total: f64,
    currency: String,
    notes: Option<String>,
    placed_at: NaiveDateTime,
    c_id: i32,
    email: String,
    first_name: String,
    last_name: String,
    phone: Option<String>,
    r_id: i32,
    r_code: String,
    r_name: String,
}

fn collect_order_rows(rows: &[sqlx::postgres::PgRow]) -> Result<Vec<OrderRow>, sqlx::Error> {
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

async fn fetch_items(pool: &PgPool, ids: &[i32]) -> Result<HashMap<i32, Vec<OrderItemDTO>>, sqlx::Error> {
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

async fn build_result(
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

async fn read_count_cache(pool: &PgPool, key: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT total FROM count_cache WHERE cache_key=$1 AND cached_at > NOW() - INTERVAL '30 days'",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn write_count_cache(pool: &PgPool, key: &str, total: i64) {
    let _ = sqlx::query(
        "INSERT INTO count_cache (cache_key,total,cached_at) VALUES ($1,$2,NOW())
         ON CONFLICT (cache_key) DO UPDATE SET total=$2, cached_at=NOW()",
    )
    .bind(key)
    .bind(total)
    .execute(pool)
    .await;
}

pub fn build_order_where(
    q: &str, status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
    qa: &mut QueryArgs,
) -> (String, bool) {
    let mut clauses: Vec<String> = vec![];
    let mut needs_region_join = false;

    if !q.trim().is_empty() {
        for tok in q.split_whitespace() {
            let p = qa.add_str(format!("%{tok}%"));
            clauses.push(format!("o.search_text ILIKE {p}"));
        }
    }
    if !status.is_empty() {
        let quoted: Vec<String> = split_trim(status)
            .iter()
            .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
            .collect();
        clauses.push(format!("o.status = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !region_code.is_empty() {
        needs_region_join = true;
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        clauses.push(format!("r.code = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !from.is_empty() {
        let p = qa.add_str(from.to_string());
        clauses.push(format!(r#"o."placedAt" >= {p}::timestamp"#));
    }
    if !to.is_empty() {
        let p = qa.add_str(to.to_string());
        clauses.push(format!(
            r#"o."placedAt" <= ({p}::date + interval '1 day' - interval '1 second')"#
        ));
    }
    if let Some(min) = min_total {
        let p = qa.add_f64(min);
        clauses.push(format!("CAST(o.total AS FLOAT8) >= {p}"));
    }
    if let Some(max) = max_total {
        let p = qa.add_f64(max);
        clauses.push(format!("CAST(o.total AS FLOAT8) <= {p}"));
    }

    if clauses.is_empty() {
        (String::new(), needs_region_join)
    } else {
        (format!("WHERE {}", clauses.join(" AND ")), needs_region_join)
    }
}

fn build_count_cache_key(
    q: &str, status: &str, region_code: &str,
    from: &str, to: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> String {
    let min_str = min_total.map(|v| format!("{v}")).unwrap_or_default();
    let max_str = max_total.map(|v| format!("{v}")).unwrap_or_default();
    format!(
        "q={}&status={}&regionCode={}&from={}&to={}&minTotal={}&maxTotal={}",
        q.trim().to_lowercase(),
        status, region_code, from, to, min_str, max_str
    )
}

fn build_order_by(sort: &str, dir: &str) -> String {
    match sort {
        "customer" => format!(r#"c."firstName" {dir}, c."lastName" {dir}, o."placedAt" DESC"#),
        "total" => format!(r#"CAST(o.total AS FLOAT8) {dir}, o."placedAt" DESC"#),
        "status" => format!(r#"o.status {dir}, o."placedAt" DESC"#),
        "id" => format!("o.id {dir}"),
        _ => format!(r#"o."placedAt" {dir}"#),
    }
}

fn flip_order_by(ob: &str) -> String {
    ob.replace(" DESC", "\x00")
        .replace(" ASC", " DESC")
        .replace('\x00', " ASC")
}

fn safe_order_sort(s: &str) -> &str {
    match s {
        "placedAt" | "total" | "status" | "customer" | "id" => s,
        _ => "placedAt",
    }
}

fn safe_order_dir(d: &str) -> &str {
    if d.eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" }
}

fn has_short_token(q: &str) -> bool {
    q.split_whitespace().any(|t| t.len() < 3)
}

pub fn split_trim(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}
