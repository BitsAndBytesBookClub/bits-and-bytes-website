mod auth;

use crate::auth::Claims;
use axum::extract::MatchedPath;
use axum::handler::HandlerWithoutStateExt;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Request, StatusCode};
use axum::response::sse::Event;
use axum::response::Sse;
use axum::routing::post;
use axum::{routing::get, Json, Router};
use futures::Stream;
use libsql::{Builder, Database};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::StreamExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{debug, info_span, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MeetingInfoJson {
    time: String,
    day: String,
    chapter: String,
    topic: String,
    project: String,
}

impl MeetingInfoJson {
    fn concatenated(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            &self.time, &self.day, &self.chapter, &self.topic, &self.project
        )
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "bits-and-bytes-web=debug,tower_http=debug,axum::rejection=trace".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // set up channels for sending data
    let (tx, _rx) = broadcast::channel::<MeetingInfoJson>(10);

    // set up turso SQLite database connection
    let url = env::var("LIBSQL_URL").expect("LIBSQL_URL must be set");
    let token = env::var("LIBSQL_AUTH_TOKEN").unwrap_or_default();

    let db = Builder::new_remote(url, token)
        .build()
        .await
        .map_err(|err| warn!("{}", err))
        .expect("cannot build db conn");
    let shared_db = Arc::new(Mutex::new(db));
    debug!("database remote connection established...");

    let not_found_svc = not_found().await.into_service();
    let root = ServeDir::new("assets").not_found_service(not_found_svc.clone());

    let app = Router::new()
        .nest_service("/", root.clone())
        .route(
            "/events",
            get({
                let tx_clone = tx.clone();
                move || {
                    let rx = tx_clone.subscribe();
                    sse_handler(rx)
                }
            }),
        )
        .route(
            "/update/meeting_info",
            post({
                let tx_clone = tx.clone();
                let db_clone = shared_db.clone();
                move |claims: Claims, Json(payload): Json<MeetingInfoJson>| {
                    update_meeting_handler(claims, payload, tx_clone.clone(), db_clone.clone())
                }
            }),
        )
        .route(
            "/current/meeting_info",
            get(move || meeting_info_handler(shared_db.clone())),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(vec![CONTENT_TYPE]),
        )
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let matched_path = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str);

                info_span!(
                    "http_request",
                    method = ?request.method(),
                    matched_path,
                    some_other_field = tracing::field::Empty,
                )
            }),
        )
        .fallback_service(not_found_svc);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    debug!("starting web server on port 3000...");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not found")
}

async fn update_meeting_handler(
    _claims: Claims,
    time_update: MeetingInfoJson,
    tx: broadcast::Sender<MeetingInfoJson>,
    db: Arc<Mutex<Database>>,
) -> Result<StatusCode, (StatusCode, String)> {
    debug!("sending POST to /update/meeting_info");
    debug!("sending tx data {:?}", time_update);

    let conn = db.lock().await.connect().map_err(|err| {
        warn!("{}", err);
        (
            StatusCode::BAD_REQUEST,
            format!("Database connection error: {}", err),
        )
    })?;

    let queries = vec![
        (
            "INSERT INTO times (time) VALUES (:value)",
            time_update.time.clone(),
        ),
        (
            "INSERT INTO days (day) VALUES (:value)",
            time_update.day.clone(),
        ),
        (
            "INSERT INTO chapters (chapter) VALUES (:value)",
            time_update.chapter.clone(),
        ),
        (
            "INSERT INTO topics (topic) VALUES (:value)",
            time_update.topic.clone(),
        ),
        (
            "INSERT INTO projects (project) VALUES (:value)",
            time_update.project.clone(),
        ),
    ];

    for (query, param) in queries {
        conn.execute(query, libsql::named_params! { ":value": param })
            .await
            .map_err(|err| {
                warn!("{}", err);
                (
                    StatusCode::BAD_REQUEST,
                    format!("Database execution error: {}", err),
                )
            })?;
    }

    tx.send(time_update).map_err(|err| {
        warn!("{}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Broadcast error: {}", err),
        )
    })?;

    Ok(StatusCode::CREATED)
}

async fn meeting_info_handler(db: Arc<Mutex<Database>>) -> Json<MeetingInfoJson> {
    debug!("sending GET to meeting_info");
    let conn = db
        .lock()
        .await
        .connect()
        .map_err(|err| warn!("{}", err))
        .expect("could not connect to turso");
    let mut data = conn
        .query("SELECT * FROM times ORDER BY ID DESC LIMIT 1;", ())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let time = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM days ORDER BY ID DESC LIMIT 1;", ())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let day = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM chapters ORDER BY ID DESC LIMIT 1;", ())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let reading_assignment = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM topics ORDER BY ID DESC LIMIT 1;", ())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let discussion_topic = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM projects ORDER BY ID DESC LIMIT 1;", ())
        .await
        .map_err(|err| warn!("{}", err))
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let optional_projects = rows.get_str(1).unwrap().to_string();

    Json(MeetingInfoJson {
        time,
        day,
        chapter: reading_assignment,
        topic: discussion_topic,
        project: optional_projects,
    })
}

async fn sse_handler(
    rx: broadcast::Receiver<MeetingInfoJson>,
) -> Sse<impl Stream<Item = Result<Event, String>> + Send + 'static> {
    debug!("sending GET to /events");
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).map(|result| match result {
        Ok(msg) => {
            debug!("receiving rx data {:?}", msg.clone());
            Ok(Event::default().data(msg.concatenated()))
        }
        Err(e) => {
            warn!("error broadcasting message {}", e);
            eprintln!("broadcast channel error: {:?}", e);
            Err("error with data".to_string())
        }
    });

    Sse::new(stream)
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
