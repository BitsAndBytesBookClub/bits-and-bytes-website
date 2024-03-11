use axum::handler::{HandlerService, HandlerWithoutStateExt};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{routing::get, Router};
use tokio::signal;
use tower_http::services::ServeDir;
use tower_http::set_status::SetStatus;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let not_found_svc = not_found().await.into_service();
    let root = ServeDir::new("assets").not_found_service(not_found_svc);

    let app = Router::new().nest_service("/", root.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("starting web server on port 3000...");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not found")
}
