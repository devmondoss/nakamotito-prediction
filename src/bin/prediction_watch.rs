//! Prediction Watch — referencia de resolución en tiempo real, sin persistencia histórica.
//!
//! Este proceso no es el motor Binance ni un ejecutor. Escucha el RTDS público de Polymarket
//! para el BTC/USD TWAP de Chainlink y publica una foto efímera para la futura consola de
//! Nakamotito. Al terminar limpia su snapshot; no escribe WAL, Parquet, DuckDB ni órdenes.
//!
//! Uso: `cargo run --release --bin prediction_watch -- --out prediction-state`

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const RTDS_URL: &str = "wss://ws-live-data.polymarket.com";
const TOPIC: &str = "crypto_prices_twap_sixty";
const SYMBOL: &str = "btc/usd";
const SNAPSHOT: &str = "prediction-watch.json";

#[derive(Clone, Debug, Serialize, PartialEq)]
struct Twap {
    symbol: String,
    window_seconds: u8,
    /// Decimal textual de presentación: nunca se degrada a `f64` en el puente.
    value: String,
    /// Entero fijo E18 cuando RTDS lo entrega; conserva la precisión exacta.
    full_accuracy_value: Option<String>,
    /// Tiempo de observación Chainlink, no el de llegada a RTDS.
    observed_at_ms: i64,
    /// Tiempo de publicación del update por RTDS.
    published_at_ms: i64,
    /// Cuando Nakamotito recibió y procesó el mensaje.
    received_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct Health {
    connected: bool,
    reconnects: u64,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Snapshot {
    schema: &'static str,
    generated_at_ms: i64,
    /// Un lector debe rechazar el snapshot si el proceso murió o dejó de refrescarlo.
    expires_at_ms: i64,
    mode: &'static str,
    execution: bool,
    persistence: &'static str,
    source: &'static str,
    chainlink_twap: Option<Twap>,
    health: Health,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

/// Acepta el payload documentado por RTDS. Rechaza otro símbolo/ventana para que una
/// suscripción cambiada o un mensaje ajeno nunca parezca el objetivo de resolución.
fn parse_twap(text: &str, received_at_ms: i64) -> Option<Twap> {
    let message: Value = serde_json::from_str(text).ok()?;
    if message.get("topic")?.as_str()? != TOPIC {
        return None;
    }
    let payload = message.get("payload")?;
    if payload.get("symbol")?.as_str()? != SYMBOL {
        return None;
    }
    let window_seconds = payload
        .get("window_s")
        .or_else(|| payload.get("windowSeconds"))?
        .as_u64()?;
    if window_seconds != 60 {
        return None;
    }
    Some(Twap {
        symbol: SYMBOL.to_string(),
        window_seconds: 60,
        value: value_text(payload.get("value")?)?,
        full_accuracy_value: payload.get("full_accuracy_value").and_then(value_text),
        observed_at_ms: payload.get("timestamp")?.as_i64()?,
        published_at_ms: message.get("timestamp")?.as_i64()?,
        received_at_ms,
    })
}

fn subscription() -> String {
    serde_json::json!({
        "action": "subscribe",
        "subscriptions": [{
            "topic": TOPIC,
            "type": "update",
            "filters": r#"{"symbol":"btc/usd"}"#,
        }],
    })
    .to_string()
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("snapshot sin directorio padre")?)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(snapshot)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn output_arg() -> PathBuf {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("prediction-state");
    while let Some(arg) = args.next() {
        if arg == "--out" {
            if let Some(value) = args.next() {
                out = PathBuf::from(value);
            }
        }
    }
    out.join(SNAPSHOT)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "prediction_watch=info".into()),
        )
        .with_target(false)
        .init();

    let output = output_arg();
    let mut twap = None;
    let mut reconnects = 0_u64;
    let mut last_error = None;
    info!("Prediction Watch · {SYMBOL} · Chainlink TWAP 60s · efímero/read-only");

    loop {
        let snapshot = Snapshot {
            schema: "prediction-watch-v1",
            generated_at_ms: now_ms(),
            expires_at_ms: now_ms() + 10_000,
            mode: "streaming-first",
            execution: false,
            persistence: "none",
            source: "polymarket-rtds/chainlink",
            chainlink_twap: twap.clone(),
            health: Health {
                connected: false,
                reconnects,
                last_error: last_error.clone(),
            },
        };
        write_snapshot(&output, &snapshot)?;

        let connected = connect_async(RTDS_URL).await;
        let (socket, _) = match connected {
            Ok(connection) => connection,
            Err(error) => {
                last_error = Some(error.to_string());
                reconnects += 1;
                warn!("RTDS no conecta: {error}; reintentando");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let (mut writer, mut reader) = socket.split();
        writer.send(Message::Text(subscription().into())).await?;
        write_snapshot(&output, &Snapshot {
            schema: "prediction-watch-v1",
            generated_at_ms: now_ms(),
            expires_at_ms: now_ms() + 10_000,
            mode: "streaming-first",
            execution: false,
            persistence: "none",
            source: "polymarket-rtds/chainlink",
            chainlink_twap: twap.clone(),
            health: Health { connected: true, reconnects, last_error: None },
        })?;
        info!("RTDS conectado");
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));

        let disconnected = loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if let Err(error) = writer.send(Message::Text("PING".into())).await {
                        break Some(error.to_string());
                    }
                }
                message = reader.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(next) = parse_twap(&text, now_ms()) {
                            twap = Some(next);
                            let snapshot = Snapshot {
                                schema: "prediction-watch-v1",
                                generated_at_ms: now_ms(),
                                expires_at_ms: now_ms() + 10_000,
                                mode: "streaming-first",
                                execution: false,
                                persistence: "none",
                                source: "polymarket-rtds/chainlink",
                                chainlink_twap: twap.clone(),
                                health: Health { connected: true, reconnects, last_error: None },
                            };
                            if let Err(error) = write_snapshot(&output, &snapshot) {
                                warn!("no se pudo publicar snapshot efímero: {error}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break Some("RTDS cerró la conexión".to_string()),
                    Some(Err(error)) => break Some(error.to_string()),
                    _ => {}
                },
                _ = tokio::signal::ctrl_c() => {
                    let _ = std::fs::remove_file(&output);
                    info!("Prediction Watch detenido; snapshot efímero eliminado");
                    return Ok(());
                }
            }
        };
        last_error = disconnected;
        reconnects += 1;
        warn!("RTDS desconectado; reintentando");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acepta_el_payload_twap_60_btc_usd() {
        let payload = r#"{"topic":"crypto_prices_twap_sixty","timestamp":1002,"payload":{"symbol":"btc/usd","value":65000.5,"full_accuracy_value":"65000500000000000000000","timestamp":1000,"window_s":60}}"#;
        assert_eq!(parse_twap(payload, 1003), Some(Twap {
            symbol: "btc/usd".to_string(), window_seconds: 60,
            value: "65000.5".to_string(),
            full_accuracy_value: Some("65000500000000000000000".to_string()),
            observed_at_ms: 1000, published_at_ms: 1002, received_at_ms: 1003,
        }));
    }

    #[test]
    fn rechaza_otra_ventana_o_simbolo() {
        let bad_window = r#"{"topic":"crypto_prices_twap_sixty","timestamp":2,"payload":{"symbol":"btc/usd","value":"1","timestamp":1,"window_s":30}}"#;
        let bad_symbol = r#"{"topic":"crypto_prices_twap_sixty","timestamp":2,"payload":{"symbol":"eth/usd","value":"1","timestamp":1,"window_s":60}}"#;
        assert!(parse_twap(bad_window, 3).is_none());
        assert!(parse_twap(bad_symbol, 3).is_none());
    }
}
