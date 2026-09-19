//! Prediction Console — panel local read-only para snapshots y captura de investigación.
//!
//! No posee una wallet, API key ni lógica de ejecución. Sus únicos procesos hijos son los dos
//! watchers de este repo; al detenerse los termina y los snapshots se eliminan.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};
use tracing::{info, warn};

const UI: &str = include_str!("../console.html");

struct Watchers {
    oracle: Option<Child>,
    clob: Option<Child>,
}

struct AppState {
    state_dir: PathBuf,
    data_dir: PathBuf,
    watchers: Mutex<Watchers>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn state_dir() -> PathBuf {
    std::env::var_os("PREDICTION_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("prediction-state"))
}

fn data_dir() -> PathBuf {
    std::env::var_os("PREDICTION_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("prediction-data"))
}

async fn read_snapshot(path: PathBuf) -> Value {
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(snapshot) => {
                let expires_at_ms = snapshot
                    .get("expires_at_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                json!({ "present": true, "fresh": expires_at_ms > now_ms(), "age_ms": now_ms() - snapshot.get("generated_at_ms").and_then(Value::as_i64).unwrap_or(0), "snapshot": snapshot })
            }
            Err(error) => {
                json!({ "present": true, "fresh": false, "error": format!("snapshot inválido: {error}") })
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({ "present": false, "fresh": false })
        }
        Err(error) => json!({ "present": false, "fresh": false, "error": error.to_string() }),
    }
}

async fn index() -> Html<&'static str> {
    Html(UI)
}

async fn snapshot(State(app): State<Arc<AppState>>) -> Json<Value> {
    let oracle_path = app.state_dir.join("prediction-watch.json");
    let clob_path = app.state_dir.join("prediction-clob-watch.json");
    let (oracle, clob) = tokio::join!(read_snapshot(oracle_path), read_snapshot(clob_path));
    let watchers = app.watchers.lock().await;
    Json(json!({
        "now_ms": now_ms(),
        "state_dir": app.state_dir,
        "data_dir": app.data_dir,
        "watchers": { "oracle": watchers.oracle.is_some(), "clob": watchers.clob.is_some() },
        "oracle": oracle,
        "clob": clob,
    }))
}

fn child(bin: &str, state_dir: &PathBuf, data_dir: &PathBuf) -> std::io::Result<Child> {
    let own_executable = std::env::current_exe()?;
    let sibling = own_executable.with_file_name(bin);
    // `cargo run` would make Cargo the direct child and can orphan the watcher when the console
    // stops. Build a missing sibling once, then own the actual watcher process directly. The
    // fallback build must match the console's own profile: `cargo build` defaults to `debug`, so
    // a release console would otherwise keep looking for a sibling in `target/release/` that the
    // build never produces there.
    if !sibling.exists() {
        let is_release = own_executable
            .parent()
            .and_then(|dir| dir.file_name())
            .is_some_and(|name| name == "release");
        let mut build = std::process::Command::new("cargo");
        build.args(["build", "--quiet", "--bin", bin]);
        if is_release {
            build.arg("--release");
        }
        let status = build.status()?;
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "cargo no pudo compilar {bin}"
            )));
        }
    }
    Command::new(sibling)
        .arg("--out")
        .arg(state_dir)
        .arg("--record")
        .arg(data_dir)
        .spawn()
}

async fn start(State(app): State<Arc<AppState>>) -> Result<Json<Value>, ConsoleError> {
    let mut watchers = app.watchers.lock().await;
    if watchers.oracle.is_none() {
        watchers.oracle = Some(
            child("prediction_watch", &app.state_dir, &app.data_dir)
                .context("no se pudo iniciar Oracle Watch")?,
        );
    }
    if watchers.clob.is_none() {
        watchers.clob = Some(
            child("prediction_clob_watch", &app.state_dir, &app.data_dir)
                .context("no se pudo iniciar CLOB Watch")?,
        );
    }
    info!(dir = %app.state_dir.display(), "watchers iniciados desde Prediction Console");
    Ok(Json(json!({ "ok": true })))
}

async fn stop(State(app): State<Arc<AppState>>) -> Json<Value> {
    let mut watchers = app.watchers.lock().await;
    stop_process(&mut watchers.oracle).await;
    stop_process(&mut watchers.clob).await;
    remove_snapshots(&app.state_dir).await;
    Json(json!({ "ok": true }))
}

async fn stop_process(process: &mut Option<Child>) {
    if let Some(child) = process.as_mut() {
        if let Err(error) = child.kill().await {
            warn!("no se pudo detener watcher: {error}");
        }
    }
    *process = None;
}

async fn shutdown(app: &AppState) {
    let mut watchers = app.watchers.lock().await;
    stop_process(&mut watchers.oracle).await;
    stop_process(&mut watchers.clob).await;
    remove_snapshots(&app.state_dir).await;
}

async fn remove_snapshots(state_dir: &PathBuf) {
    for filename in ["prediction-watch.json", "prediction-clob-watch.json"] {
        match tokio::fs::remove_file(state_dir.join(filename)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!("no se pudo eliminar snapshot efímero: {error}"),
        }
    }
}

struct ConsoleError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ConsoleError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ConsoleError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({ "ok": false, "error": self.0.to_string() })),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "prediction_console=info".into()),
        )
        .with_target(false)
        .init();

    let app = Arc::new(AppState {
        state_dir: state_dir(),
        data_dir: data_dir(),
        watchers: Mutex::new(Watchers {
            oracle: None,
            clob: None,
        }),
    });
    let router = Router::new()
        .route("/", get(index))
        .route("/api/snapshot", get(snapshot))
        .route("/api/watchers/start", post(start))
        .route("/api/watchers/stop", post(stop))
        .with_state(app.clone());
    let address = "127.0.0.1:3090";
    let listener = tokio::net::TcpListener::bind(address).await?;
    info!("Prediction Console disponible en http://{address}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown(&app).await;
        })
        .await?;
    Ok(())
}
