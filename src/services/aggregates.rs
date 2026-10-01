use crate::model::{CategoryAggregateDTO, DailyAggregateDTO, TotalsDTO};
use crate::services::orders::{exact_count_uncapped, split_trim};
use crate::services::query::QueryArgs;
use sqlx::{Arguments, PgPool, Row};
use std::collections::HashMap;

pub async fn get_exact_total(
    pool: &PgPool,
    from: &str, to: &str,
    q: &str, status: &str, region_code: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<i64, sqlx::Error> {
    exact_count_uncapped(pool, q, status, region_code, from, to, min_total, max_total).await
}

pub async fn get_daily_aggregates(
    pool: &PgPool,
    from: &str, to: &str,
    q: &str, status: &str, region_code: &str,
    min_total: Option<f64>, max_total: Option<f64>,
    top_categories: i32,
) -> Result<Vec<DailyAggregateDTO>, sqlx::Error> {
    let has_q = !q.trim().is_empty();
    let is_multi_token = has_q && q.trim().contains(' ');
    let has_status = !status.trim().is_empty();
    let has_region = !region_code.trim().is_empty();
    let has_total = min_total.is_some() || max_total.is_some();

    let rows = match (has_q, is_multi_token, has_status, has_region, has_total) {
        (false, _, false, false, false) => query_daily_summary(pool, from, to, region_code).await?,
        (true, true, _, _, false) => {
            let rows = query_multi_token_via_cte(pool, from, to, q, status, region_code).await?;
            if rows.is_empty() {
                query_via_search_text(pool, from, to, q, status, region_code, min_total, max_total).await?
            } else {
                rows
            }
        }
        (true, _, _, _, _) => {
            query_via_search_text(pool, from, to, q, status, region_code, min_total, max_total).await?
        }
        (false, _, true, false, false) => query_status_category_summary(pool, from, to, status).await?,
        (false, _, _, _, false) if has_status || has_region => {
            query_filter_category_summary(pool, from, to, status, region_code).await?
        }
        _ => query_order_category_facts(pool, from, to, status, region_code, min_total, max_total).await?,
    };

    let top_n = if top_categories <= 0 { 5 } else { top_categories };
    Ok(build_aggregate_result(rows, top_n))
}

#[derive(Default)]
struct AggRow {
    day: String,
    category: String,
    orders: i64,
    revenue: f64,
    items: i64,
}

async fn query_agg_rows(
    pool: &PgPool,
    sql: &str,
    args: sqlx::postgres::PgArguments,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let rows = sqlx::query_with(sql, args).fetch_all(pool).await?;
    rows.iter()
        .map(|r| {
            Ok(AggRow {
                day: r.try_get("day")?,
                category: r.try_get("category")?,
                orders: r.try_get("total_orders")?,
                revenue: r.try_get("total_revenue")?,
                items: r.try_get("total_items")?,
            })
        })
        .collect()
}

async fn query_daily_summary(
    pool: &PgPool,
    from: &str, to: &str, region_code: &str,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let mut extra = String::new();
    if !region_code.is_empty() {
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        extra = format!(r#" AND "regionCode" = ANY(ARRAY[{}])"#, quoted.join(","));
    }
    let sql = format!(
        r#"SELECT date::text AS day, "categoryName" AS category,
                  SUM("totalOrders")::bigint AS total_orders, CAST(SUM("totalRevenue") AS FLOAT8) AS total_revenue,
                  SUM("totalItems")::bigint AS total_items
           FROM daily_summary
           WHERE date BETWEEN $1::date AND $2::date{extra}
           GROUP BY date, "categoryName" ORDER BY date"#
    );
    let mut args = sqlx::postgres::PgArguments::default();
    let _ = args.add(from.to_string());
    let _ = args.add(to.to_string());
    query_agg_rows(pool, &sql, args).await
}

async fn query_status_category_summary(
    pool: &PgPool,
    from: &str, to: &str, status: &str,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let quoted: Vec<String> = split_trim(status)
        .iter()
        .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
        .collect();
    let sql = format!(
        r#"SELECT date::text AS day, "categoryName" AS category,
                  SUM("totalOrders")::bigint AS total_orders, CAST(SUM("totalRevenue") AS FLOAT8) AS total_revenue,
                  SUM("totalItems")::bigint AS total_items
           FROM daily_status_category_summary
           WHERE status = ANY(ARRAY[{}])
           AND date BETWEEN $1::date AND $2::date
           GROUP BY date, "categoryName" ORDER BY date"#,
        quoted.join(",")
    );
    let mut args = sqlx::postgres::PgArguments::default();
    let _ = args.add(from.to_string());
    let _ = args.add(to.to_string());
    query_agg_rows(pool, &sql, args).await
}

async fn query_filter_category_summary(
    pool: &PgPool,
    from: &str, to: &str, status: &str, region_code: &str,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let mut extra: Vec<String> = vec![];
    if !status.is_empty() {
        let quoted: Vec<String> = split_trim(status)
            .iter()
            .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
            .collect();
        extra.push(format!("status = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !region_code.is_empty() {
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        extra.push(format!(r#""regionCode" = ANY(ARRAY[{}])"#, quoted.join(",")));
    }
    let extra_str = if extra.is_empty() {
        String::new()
    } else {
        format!(" AND {}", extra.join(" AND "))
    };
    let sql = format!(
        r#"SELECT date::text AS day, "categoryName" AS category,
                  SUM("totalOrders")::bigint AS total_orders, CAST(SUM("totalRevenue") AS FLOAT8) AS total_revenue,
                  SUM("totalItems")::bigint AS total_items
           FROM daily_filter_category_summary
           WHERE date BETWEEN $1::date AND $2::date{extra_str}
           GROUP BY date, "categoryName" ORDER BY date"#
    );
    let mut args = sqlx::postgres::PgArguments::default();
    let _ = args.add(from.to_string());
    let _ = args.add(to.to_string());
    query_agg_rows(pool, &sql, args).await
}

async fn query_order_category_facts(
    pool: &PgPool,
    from: &str, to: &str, status: &str, region_code: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let mut qa = QueryArgs::new();
    let from_p = qa.add_str(from.to_string());
    let to_p = qa.add_str(to.to_string());
    let mut extra: Vec<String> = vec![];
    if !status.is_empty() {
        let quoted: Vec<String> = split_trim(status)
            .iter()
            .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
            .collect();
        extra.push(format!("status = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !region_code.is_empty() {
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        extra.push(format!(r#""regionCode" = ANY(ARRAY[{}])"#, quoted.join(",")));
    }
    if let Some(min) = min_total {
        let p = qa.add_f64(min);
        extra.push(format!(r#""orderTotal" >= {p}"#));
    }
    if let Some(max) = max_total {
        let p = qa.add_f64(max);
        extra.push(format!(r#""orderTotal" <= {p}"#));
    }
    let extra_str = if extra.is_empty() {
        String::new()
    } else {
        format!(" AND {}", extra.join(" AND "))
    };
    let sql = format!(
        r#"SELECT date::text AS day, "categoryName" AS category,
                  COUNT(DISTINCT "orderId")::bigint AS total_orders, CAST(SUM("totalRevenue") AS FLOAT8) AS total_revenue,
                  SUM("totalItems")::bigint AS total_items
           FROM order_category_facts
           WHERE date BETWEEN {from_p}::date AND {to_p}::date{extra_str}
           GROUP BY date, "categoryName" ORDER BY date"#
    );
    query_agg_rows(pool, &sql, qa.into_args()).await
}

async fn query_via_search_text(
    pool: &PgPool,
    from: &str, to: &str,
    q: &str, status: &str, region_code: &str,
    min_total: Option<f64>, max_total: Option<f64>,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let mut qa = QueryArgs::new();
    let from_p = qa.add_str(from.to_string());
    let to_p = qa.add_str(to.to_string());
    let mut clauses: Vec<String> = vec![];
    for tok in q.split_whitespace() {
        let p = qa.add_str(format!("%{tok}%"));
        clauses.push(format!("o.search_text ILIKE {p}"));
    }
    if !status.is_empty() {
        let quoted: Vec<String> = split_trim(status)
            .iter()
            .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
            .collect();
        clauses.push(format!("o.status = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !region_code.is_empty() {
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        clauses.push(format!("r.code = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if let Some(min) = min_total {
        let p = qa.add_f64(min);
        clauses.push(format!("CAST(o.total AS FLOAT8) >= {p}"));
    }
    if let Some(max) = max_total {
        let p = qa.add_f64(max);
        clauses.push(format!("CAST(o.total AS FLOAT8) <= {p}"));
    }
    let extra = if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", clauses.join(" AND "))
    };
    let sql = format!(
        r#"SELECT o."placedAt"::date::text AS day, cat.name AS category,
                  COUNT(DISTINCT o.id)::bigint AS total_orders,
                  COALESCE(CAST(SUM(oi.quantity * CAST(oi."unitPrice" AS FLOAT8) * (1 - CAST(oi.discount AS FLOAT8))) AS FLOAT8),0) AS total_revenue,
                  COALESCE(SUM(oi.quantity),0)::bigint AS total_items
           FROM orders o
           JOIN customers c ON c.id = o."customerId"
           JOIN regions r ON r.id = o."regionId"
           JOIN order_items oi ON oi."orderId" = o.id
           JOIN products p ON p.id = oi."productId"
           JOIN categories cat ON cat.id = p."categoryId"
           WHERE o."placedAt"::date BETWEEN {from_p}::date AND {to_p}::date{extra}
           GROUP BY o."placedAt"::date, cat.name ORDER BY o."placedAt"::date"#
    );
    query_agg_rows(pool, &sql, qa.into_args()).await
}

async fn query_multi_token_via_cte(
    pool: &PgPool,
    from: &str, to: &str,
    q: &str, status: &str, region_code: &str,
) -> Result<Vec<AggRow>, sqlx::Error> {
    let mut qa = QueryArgs::new();
    let from_p = qa.add_str(from.to_string());
    let to_p = qa.add_str(to.to_string());
    let mut token_clauses: Vec<String> = vec![];
    for tok in q.split_whitespace() {
        let p = qa.add_str(format!("%{tok}%"));
        token_clauses.push(format!(r#"("firstName" || ' ' || "lastName") ILIKE {p}"#));
    }
    let mut extra: Vec<String> = vec![];
    if !status.is_empty() {
        let quoted: Vec<String> = split_trim(status)
            .iter()
            .map(|s| format!("'{}'::\"OrderStatus\"", s.replace('\'', "''")))
            .collect();
        extra.push(format!("dcs.status = ANY(ARRAY[{}])", quoted.join(",")));
    }
    if !region_code.is_empty() {
        let quoted: Vec<String> = split_trim(region_code)
            .iter()
            .map(|c| format!("'{}'", c.replace('\'', "''")))
            .collect();
        extra.push(format!(r#"dcs."regionCode" = ANY(ARRAY[{}])"#, quoted.join(",")));
    }
    let extra_str = if extra.is_empty() {
        String::new()
    } else {
        format!(" AND {}", extra.join(" AND "))
    };
    let sql = format!(
        r#"WITH matching_customers AS (
             SELECT id FROM customers WHERE {token_where}
           )
           SELECT dcs.date::text AS day, dcs."categoryName" AS category,
                  SUM(dcs."totalOrders")::bigint AS total_orders,
                  CAST(SUM(dcs."totalRevenue") AS FLOAT8) AS total_revenue,
                  SUM(dcs."totalItems")::bigint AS total_items
           FROM daily_customer_category_summary dcs
           WHERE dcs."customerId" IN (SELECT id FROM matching_customers)
           AND dcs.date BETWEEN {from_p}::date AND {to_p}::date{extra_str}
           GROUP BY dcs.date, dcs."categoryName" ORDER BY dcs.date"#,
        token_where = token_clauses.join(" AND ")
    );
    query_agg_rows(pool, &sql, qa.into_args()).await
}

fn build_aggregate_result(rows: Vec<AggRow>, top_n: i32) -> Vec<DailyAggregateDTO> {
    let top_n = top_n as usize;
    struct DayEntry {
        cats: HashMap<String, [f64; 3]>,
    }
    let mut by_day: HashMap<String, DayEntry> = HashMap::new();
    let mut day_order: Vec<String> = vec![];

    for r in &rows {
        let entry = by_day.entry(r.day.clone()).or_insert_with(|| {
            day_order.push(r.day.clone());
            DayEntry { cats: HashMap::new() }
        });
        let v = entry.cats.entry(r.category.clone()).or_insert([0.0; 3]);
        v[0] += r.orders as f64;
        v[1] += r.revenue;
        v[2] += r.items as f64;
    }

    let mut cat_totals: HashMap<String, f64> = HashMap::new();
    for de in by_day.values() {
        for (cat, v) in &de.cats {
            *cat_totals.entry(cat.clone()).or_insert(0.0) += v[0];
        }
    }
    let mut sorted_cats: Vec<(String, f64)> = cat_totals.into_iter().collect();
    sorted_cats.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_set: std::collections::HashSet<String> = sorted_cats
        .iter()
        .take(top_n)
        .map(|(k, _)| k.clone())
        .collect();

    day_order
        .iter()
        .map(|day| {
            let de = &by_day[day];
            let mut cats: HashMap<String, CategoryAggregateDTO> = HashMap::new();
            let mut oth_o = 0.0f64;
            let mut oth_r = 0.0f64;
            let mut oth_i = 0.0f64;
            for (cat, v) in &de.cats {
                let (o, r, i) = (v[0], v[1], v[2]);
                if top_set.contains(cat.as_str()) {
                    let avg = if o > 0.0 { r / o } else { 0.0 };
                    cats.insert(
                        cat.clone(),
                        CategoryAggregateDTO {
                            total_orders: o as i64,
                            total_revenue: r,
                            total_items: i as i64,
                            avg_order_value: (avg * 100.0).round() / 100.0,
                        },
                    );
                } else {
                    oth_o += o;
                    oth_r += r;
                    oth_i += i;
                }
            }
            if oth_o > 0.0 {
                let avg = oth_r / oth_o;
                cats.insert(
                    "Others".to_string(),
                    CategoryAggregateDTO {
                        total_orders: oth_o as i64,
                        total_revenue: oth_r,
                        total_items: oth_i as i64,
                        avg_order_value: (avg * 100.0).round() / 100.0,
                    },
                );
            }
            let (tot_o, tot_r, tot_i) = cats.values().fold((0i64, 0.0f64, 0i64), |acc, v| {
                (acc.0 + v.total_orders, acc.1 + v.total_revenue, acc.2 + v.total_items)
            });
            DailyAggregateDTO {
                date: day.clone(),
                categories: cats,
                totals: TotalsDTO {
                    total_orders: tot_o,
                    total_revenue: tot_r,
                    total_items: tot_i,
                },
            }
        })
        .collect()
}
