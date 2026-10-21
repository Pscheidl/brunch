use brunch_server::{AppState, Config, app, auth, commerce, mail, migrate};
use sqlx::postgres::PgPoolOptions;
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "brunch_server=info".into()),
        )
        .init();
    let config = Arc::new(Config::from_env()?);
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await?;
    migrate(&pool).await?;
    commerce::seed(&pool, &config.order_email).await?;
    auth::bootstrap_admin(&pool, &config.admin_email).await?;
    let state = AppState {
        pool,
        config: config.clone(),
    };
    let transport = mail::transport(&config)?;
    let worker_state = state.clone();
    let worker = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            if let Err(error) = mail::deliver_batch(&worker_state, &transport).await {
                tracing::error!(%error,"Mail queue unavailable");
            }
            if let Err(error) = auth::cleanup(&worker_state.pool).await {
                tracing::error!(%error,"Session cleanup unavailable");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address=%config.bind,"Brunch server ready");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    worker.abort();
    Ok(())
}
