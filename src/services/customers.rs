use crate::model::{CustomerDTO, CustomerListResult, RegionDTO};
use crate::services::query::QueryArgs;
use sqlx::{PgPool, Row};

pub async fn list_customers(
    pool: &PgPool,
    cursor: Option<i32>,
    limit: i32,
    q: &str,
    region_id: Option<i32>,
) -> Result<CustomerListResult, sqlx::Error> {
    let limit = limit.max(1).min(100);
    let mut qa = QueryArgs::new();
    let mut clauses: Vec<String> = vec![];

    if let Some(c) = cursor {
        let p = qa.add_i32(c);
        clauses.push(format!("c.id > {p}"));
    }
    if !q.trim().is_empty() {
        let p = qa.add_str(format!("%{}%", q.trim()));
        clauses.push(format!(
            r#"(c."firstName" || ' ' || c."lastName" || ' ' || c.email) ILIKE {p}"#
        ));
    }
    if let Some(rid) = region_id {
        let p = qa.add_i32(rid);
        clauses.push(format!(r#"c."regionId" = {p}"#));
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let lp = qa.add_i32(limit + 1);

    let sql = format!(
        r#"SELECT c.id, c.email, c."firstName", c."lastName", c.phone,
                  c."createdAt"::text,
                  r.id AS r_id, r.code, r.name AS r_name
           FROM customers c
           JOIN regions r ON r.id = c."regionId"
           {where_clause}
           ORDER BY c.id LIMIT {lp}"#
    );

    let rows = sqlx::query_with(&sql, qa.into_args())
        .fetch_all(pool)
        .await?;

    let mut data: Vec<CustomerDTO> = rows
        .iter()
        .map(|r| CustomerDTO {
            id: r.try_get("id").unwrap_or(0),
            email: r.try_get("email").unwrap_or_default(),
            first_name: r.try_get("firstName").unwrap_or_default(),
            last_name: r.try_get("lastName").unwrap_or_default(),
            phone: r.try_get("phone").unwrap_or(None),
            created_at: r.try_get("createdAt").unwrap_or(None),
            region: RegionDTO {
                id: r.try_get("r_id").unwrap_or(0),
                code: r.try_get("code").unwrap_or_default(),
                name: r.try_get("r_name").unwrap_or_default(),
            },
        })
        .collect();

    let has_more = data.len() > limit as usize;
    if has_more {
        data.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        data.last().map(|d| d.id)
    } else {
        None
    };

    Ok(CustomerListResult {
        data,
        next_cursor,
        has_more,
    })
}
