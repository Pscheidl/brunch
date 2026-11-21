use brunch_server::{AppState, Config, app, auth, commerce, mail, migrate, payments};
use sqlx::postgres::PgPoolOptions;
use std::{
    io::{IsTerminal, Read},
    sync::Arc,
    time::Duration,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (check_config, initialize_email) = match args.as_slice() {
        [] => (false, None),
        [arg] if arg == "--check-config" => (true, None),
        [arg, email] if arg == "--initialize-admin" => (false, Some(email.as_str())),
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "Usage: brunch-server [--check-config | --initialize-admin EMAIL]\n--check-config validates settings without connecting to PostgreSQL, SMTP or Teya.\n--initialize-admin creates the first database administrator only when none can sign in. Supply the password through non-terminal standard input, never as an argument or environment variable."
            );
            return Ok(());
        }
        _ => return Err("Unknown argument. Use brunch-server --help".into()),
    };
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "brunch_server=info".into()),
        )
        .init();
    let config = Arc::new(Config::from_env()?);
    if check_config {
        println!(
            "Configuration is valid (APP_ENV={}, Teya={}).",
            config.app_env.as_str(),
            if config.teya.is_some() {
                "enabled"
            } else {
                "disabled"
            }
        );
        return Ok(());
    }
    let initialize_password = if let Some(email) = initialize_email {
        auth::normalize_email(email)?;
        if std::io::stdin().is_terminal() {
            return Err(
                "Read the password with a hidden-input prompt and pipe it to standard input."
                    .into(),
            );
        }
        let mut password = String::new();
        std::io::stdin().take(514).read_to_string(&mut password)?;
        if password.ends_with('\n') {
            password.pop();
            if password.ends_with('\r') {
                password.pop();
            }
        }
        auth::validate_admin_password(&password)?;
        Some(password)
    } else {
        None
    };
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await?;
    migrate(&pool).await?;
    if let (Some(email), Some(password)) = (initialize_email, initialize_password) {
        auth::initialize_admin(&pool, email, &password).await?;
        println!(
            "Initial database administrator created. Further accounts are managed in Administrátoři."
        );
        return Ok(());
    }
    commerce::initialize(&pool, &config.order_email, config.app_env.is_development()).await?;
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
    let expiry_state = state.clone();
    let expiry_worker = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(error) = payments::expire_orders(&expiry_state).await {
                tracing::error!(%error, "Online order expiry unavailable");
            }
        }
    });
    let payment_state = state.clone();
    let payment_worker = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(error) = payments::process_batch(&payment_state).await {
                tracing::error!(%error, "Payment queue unavailable");
            }
        }
    });
    tracing::info!(address=%config.bind,"Brunch Era server ready");
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    worker.abort();
    payment_worker.abort();
    expiry_worker.abort();
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Cannot install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown requested, completing active requests");
}
