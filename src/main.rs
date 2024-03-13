mod auth;

use crate::auth::Claims;
use axum::handler::HandlerWithoutStateExt;
use axum::http::StatusCode;
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
use tower_http::services::ServeDir;
use tracing::info;

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
    // set up channels for sending data
    let (tx, _rx) = broadcast::channel::<MeetingInfoJson>(10);

    tracing_subscriber::fmt::init();

    // set up turso SQLite database connection
    let url = env::var("LIBSQL_URL").expect("LIBSQL_URL must be set");
    let token = env::var("LIBSQL_AUTH_TOKEN").unwrap_or_default();

    let db = Builder::new_remote(url, token).build().await.unwrap();
    let shared_db = Arc::new(Mutex::new(db));

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
        .fallback_service(not_found_svc);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("starting web server on port 3000...");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
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
) -> StatusCode {
    info!("sending tx data {:?}", time_update);
    let conn = db
        .lock()
        .await
        .connect()
        .expect("could not connect to turso");
    conn.execute(
        "INSERT INTO times (time) VALUES (:time)",
        libsql::named_params! { ":time": time_update.time.clone() },
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO days (day) VALUES (:day)",
        libsql::named_params! { ":day": time_update.day.clone() },
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO chapters (chapter) VALUES (:chapter)",
        libsql::named_params! { ":chapter": time_update.chapter.clone() },
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO topics (topic) VALUES (:topic)",
        libsql::named_params! { ":topic": time_update.day.clone() },
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO projects (project) VALUES (:project)",
        libsql::named_params! { ":project": time_update.day.clone() },
    )
    .await
    .unwrap();

    let _ = tx
        .send(time_update)
        .expect("could not send payload via channel");

    StatusCode::CREATED
}

async fn meeting_info_handler(db: Arc<Mutex<Database>>) -> Json<MeetingInfoJson> {
    let conn = db
        .lock()
        .await
        .connect()
        .expect("could not connect to turso");
    let mut data = conn
        .query("SELECT * FROM times ORDER BY ID DESC LIMIT 1;", ())
        .await
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let time = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM days ORDER BY ID DESC LIMIT 1;", ())
        .await
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let day = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM chapters ORDER BY ID DESC LIMIT 1;", ())
        .await
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let reading_assignment = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM topics ORDER BY ID DESC LIMIT 1;", ())
        .await
        .unwrap();
    let rows = data.next().await.unwrap().unwrap();
    let discussion_topic = rows.get_str(1).unwrap().to_string();

    let mut data = conn
        .query("SELECT * FROM projects ORDER BY ID DESC LIMIT 1;", ())
        .await
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
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).map(|result| match result {
        Ok(msg) => {
            info!("receiving rx data {:?}", msg.clone());
            Ok(Event::default().data(msg.concatenated()))
        }
        Err(e) => {
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
