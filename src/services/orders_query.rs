use crate::services::query::QueryArgs;

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
        clauses.push(format!(r#"o."placedAt" >= {p}::timestamptz"#));
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

pub(crate) fn build_count_cache_key(
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

pub(crate) fn build_order_by(sort: &str, dir: &str) -> String {
    match sort {
        "customer" => format!(r#"c."firstName" {dir}, c."lastName" {dir}, o."placedAt" DESC"#),
        "total" => format!(r#"CAST(o.total AS FLOAT8) {dir}, o."placedAt" DESC"#),
        "status" => format!(r#"o.status {dir}, o."placedAt" DESC"#),
        "id" => format!("o.id {dir}"),
        _ => format!(r#"o."placedAt" {dir}"#),
    }
}

pub(crate) fn flip_order_by(ob: &str) -> String {
    ob.replace(" DESC", "\x00")
        .replace(" ASC", " DESC")
        .replace('\x00', " ASC")
}

pub(crate) fn safe_order_sort(s: &str) -> &str {
    match s {
        "placedAt" | "total" | "status" | "customer" | "id" => s,
        _ => "placedAt",
    }
}

pub(crate) fn safe_order_dir(d: &str) -> &str {
    if d.eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" }
}

pub(crate) fn has_short_token(q: &str) -> bool {
    q.split_whitespace().any(|t| t.len() < 3)
}

pub fn split_trim(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}
