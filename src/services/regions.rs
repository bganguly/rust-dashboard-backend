use crate::model::RegionDTO;
use sqlx::{PgPool, Row};

pub async fn list_regions(pool: &PgPool) -> Result<Vec<RegionDTO>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, code, name FROM regions")
        .fetch_all(pool)
        .await?;

    let mut out: Vec<RegionDTO> = rows
        .iter()
        .map(|r| RegionDTO {
            id: r.try_get("id").unwrap_or(0),
            code: r.try_get("code").unwrap_or_default(),
            name: r.try_get("name").unwrap_or_default(),
        })
        .collect();

    out.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(out)
}
