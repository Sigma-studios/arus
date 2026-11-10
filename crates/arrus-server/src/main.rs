use std::net::SocketAddr;

use arrus_server::{Config, config, healthcheck};

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        let port = match config::port_from_env() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("healthcheck: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        return match healthcheck::probe(port) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("healthcheck failed: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(run()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            eprintln!("error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!(
        "data dir {}, storage limit {} bytes, format key {}",
        config.data_dir.display(),
        config.storage_limit,
        if config.format_key.is_some() { "loaded" } else { "missing" }
    );
    let state = arrus_server::AppState::open(config)?;
    let app = arrus_server::app(state.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on http://{addr}");
    let served = axum::serve(listener, app).with_graceful_shutdown(shutdown()).await;
    // Whatever happened, keep the last live changes.
    arrus_server::flush(&state).await;
    served?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}
