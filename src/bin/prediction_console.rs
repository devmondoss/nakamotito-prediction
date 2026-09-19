//! Prediction Console — panel local read-only para snapshots y captura de investigación.
//!
//! No posee una wallet, API key ni lógica de ejecución. Sus únicos procesos hijos son los
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
    collections::HashMap,
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

struct WatcherDef {
    /// Clave usada en la respuesta JSON (`watchers.<key>`, `<key>` del snapshot).
    key: &'static str,
    bin: &'static str,
    snapshot_file: &'static str,
}

/// Los seis watchers que la consola puede supervisar: el oracle y el CLOB de
/// Polymarket (settlement y mercado de predicción), más el multi-venue mínimo de la
/// fase 3 (formación de precio en Binance/Bybit, spot y perp). Ninguno ejecuta órdenes.
const WATCHER_DEFS: &[WatcherDef] = &[
    WatcherDef {
        key: "oracle",
        bin: "prediction_watch",
        snapshot_file: "prediction-watch.json",
    },
    WatcherDef {
        key: "clob",
        bin: "prediction_clob_watch",
        snapshot_file: "prediction-clob-watch.json",
    },
    WatcherDef {
        key: "binance_spot",
        bin: "prediction_binance_spot_watch",
        snapshot_file: "prediction-binance-spot-watch.json",
    },
    WatcherDef {
        key: "binance_perp",
        bin: "prediction_binance_perp_watch",
        snapshot_file: "prediction-binance-perp-watch.json",
    },
    WatcherDef {
        key: "bybit_spot",
        bin: "prediction_bybit_spot_watch",
        snapshot_file: "prediction-bybit-spot-watch.json",
    },
    WatcherDef {
        key: "bybit_perp",
        bin: "prediction_bybit_perp_watch",
        snapshot_file: "prediction-bybit-perp-watch.json",
    },
];

struct AppState {
    state_dir: PathBuf,
    data_dir: PathBuf,
    children: Mutex<HashMap<&'static str, Child>>,
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
    let snapshots = futures_util::future::join_all(
        WATCHER_DEFS
            .iter()
            .map(|def| read_snapshot(app.state_dir.join(def.snapshot_file))),
    )
    .await;
    let children = app.children.lock().await;
    let mut feeds = serde_json::Map::new();
    let mut running = serde_json::Map::new();
    for (def, snap) in WATCHER_DEFS.iter().zip(snapshots) {
        feeds.insert(def.key.to_string(), snap);
        running.insert(def.key.to_string(), json!(children.contains_key(def.key)));
    }
    Json(json!({
        "now_ms": now_ms(),
        "state_dir": app.state_dir,
        "data_dir": app.data_dir,
        "watchers": running,
        "feeds": feeds,
    }))
}

fn spawn_child(bin: &str, state_dir: &PathBuf, data_dir: &PathBuf) -> std::io::Result<Child> {
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
    let mut children = app.children.lock().await;
    for def in WATCHER_DEFS {
        if !children.contains_key(def.key) {
            let child = spawn_child(def.bin, &app.state_dir, &app.data_dir)
                .with_context(|| format!("no se pudo iniciar {}", def.bin))?;
            children.insert(def.key, child);
        }
    }
    info!(dir = %app.state_dir.display(), "watchers iniciados desde Prediction Console");
    Ok(Json(json!({ "ok": true })))
}

async fn stop(State(app): State<Arc<AppState>>) -> Json<Value> {
    let mut children = app.children.lock().await;
    for (_, mut child) in children.drain() {
        if let Err(error) = child.kill().await {
            warn!("no se pudo detener watcher: {error}");
        }
    }
    remove_snapshots(&app.state_dir).await;
    Json(json!({ "ok": true }))
}

async fn shutdown(app: &AppState) {
    let mut children = app.children.lock().await;
    for (_, mut child) in children.drain() {
        if let Err(error) = child.kill().await {
            warn!("no se pudo detener watcher: {error}");
        }
    }
    remove_snapshots(&app.state_dir).await;
}

async fn remove_snapshots(state_dir: &PathBuf) {
    for def in WATCHER_DEFS {
        match tokio::fs::remove_file(state_dir.join(def.snapshot_file)).await {
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
        children: Mutex::new(HashMap::new()),
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
        .with_graceful_shutdown(shutdown_on_signal(app))
        .await?;
    Ok(())
}

/// Escuchar solo Ctrl-C (SIGINT) deja huérfanos a los watchers hijos si a la consola la
/// para otra cosa: un `kill` simple, un supervisor de procesos o systemd mandan SIGTERM
/// por defecto, y sin este manejador la consola muere sin ejecutar `shutdown` — los
/// watchers siguen corriendo y capturando sin que nadie los vea. Ocurrió en esta misma
/// sesión: una consola de prueba matada con `kill` dejó un watcher del CLOB capturando
/// sin supervisión durante horas.
async fn shutdown_on_signal(app: Arc<AppState>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => warn!("no se pudo instalar el manejador de SIGTERM: {error}"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    shutdown(&app).await;
}
