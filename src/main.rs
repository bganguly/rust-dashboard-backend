use actix_cors::Cors;
use actix_web::{web, App, HttpServer};
use std::env;

mod cache;
mod db;
mod handlers;
mod migrate;
mod model;
mod services;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let _ = dotenvy::dotenv();
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    let pool = db::create_pool().await.expect("db connect");

    let migrations_dir = env::var("MIGRATIONS_DIR")
        .unwrap_or_else(|_| "migrations".to_string());
    migrate::run(&pool, &migrations_dir)
        .await
        .expect("migrations failed");

    let agg_cache = web::Data::new(cache::AggregatesCache::new());
    let pool_data = web::Data::new(pool.clone());

    let cache_warmup = agg_cache.clone();
    let pool_warmup = pool.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        warmup_cache(pool_warmup, cache_warmup).await;
    });

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    log::info!(
        "rust-dashboard-backend listening on :{port} (Rust {})",
        env!("CARGO_PKG_VERSION")
    );

    HttpServer::new(move || {
        let allow_origin = env::var("CORS_ORIGIN")
            .unwrap_or_else(|_| "http://localhost:5173".to_string());

        let cors = if allow_origin == "*" {
            Cors::default()
                .allow_any_origin()
                .allowed_methods(vec!["GET", "POST", "OPTIONS"])
                .allowed_headers(vec![
                    actix_web::http::header::CONTENT_TYPE,
                    actix_web::http::header::AUTHORIZATION,
                ])
                .max_age(3600)
        } else {
            Cors::default()
                .allowed_origin(&allow_origin)
                .allowed_methods(vec!["GET", "POST", "OPTIONS"])
                .allowed_headers(vec![
                    actix_web::http::header::CONTENT_TYPE,
                    actix_web::http::header::AUTHORIZATION,
                ])
                .max_age(3600)
        };

        App::new()
            .wrap(cors)
            .app_data(pool_data.clone())
            .app_data(agg_cache.clone())
            .app_data(
                web::JsonConfig::default()
                    .error_handler(|err, _req| {
                        let msg = err.to_string();
                        actix_web::error::InternalError::from_response(
                            err,
                            actix_web::HttpResponse::BadRequest()
                                .json(serde_json::json!({"error": msg})),
                        )
                        .into()
                    }),
            )
            .service(
                web::scope("/api")
                    .route("/runtime", web::get().to(handlers::runtime::runtime))
                    .route("/status", web::get().to(handlers::runtime::status))
                    .route("/seed-stats", web::get().to(handlers::runtime::seed_stats))
                    .route("/orders", web::get().to(handlers::orders::list))
                    .route("/orders", web::post().to(handlers::orders::create))
                    .route("/orders/count", web::get().to(handlers::orders::count))
                    .route("/aggregates", web::get().to(handlers::aggregates::get))
                    .route("/customers", web::get().to(handlers::customers::list))
                    .route("/regions", web::get().to(handlers::regions::list)),
            )
    })
    .bind(&addr)?
    .run()
    .await
}

async fn warmup_cache(pool: sqlx::PgPool, cache: web::Data<cache::AggregatesCache>) {
    let now = chrono::Utc::now();
    let ranges = [
        (now - chrono::Duration::days(30), now),
        (now - chrono::Duration::days(90), now),
        (now - chrono::Duration::days(180), now),
        (now - chrono::Duration::days(365), now),
    ];

    for (from_dt, to_dt) in &ranges {
        let from = from_dt.format("%Y-%m-%d").to_string();
        let to = to_dt.format("%Y-%m-%d").to_string();
        let key = cache::cache_key(&from, &to, 5);

        if cache.get(&key).is_some() {
            continue;
        }

        let data = match services::aggregates::get_daily_aggregates(
            &pool, &from, &to, "", "", "", None, None, 5,
        )
        .await
        {
            Ok(d) => d,
            Err(_) => continue,
        };

        let raw_total = match services::orders::exact_count_uncapped(
            &pool, "", "", "", &from, &to, None, None,
        )
        .await
        {
            Ok(t) => t,
            Err(_) => continue,
        };

        let cached = serde_json::json!({
            "data": data,
            "totalOrders": services::orders::adjust_count(raw_total),
            "totalOrdersApproximate": services::orders::is_approximate(raw_total),
        });

        cache.put(key, cached);
    }
}
