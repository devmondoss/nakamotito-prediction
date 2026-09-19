//! Bybit Spot Watch — BTC/USDT top-of-book y trades, sin ejecución.
//!
//! Collector de la fase 3 (multi-venue) del plan cuantitativo: formación de precio en
//! el spot de Bybit, en paralelo a Binance spot y al oracle Chainlink. No sustituye ni
//! reproduce el TWAP; es un insumo para nowcasting y microestructura. Con `--record`
//! conserva los payloads RAW para investigación reproducible.

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use nakamotito_prediction::research::{now_ms, ResearchWriter};
use nakamotito_prediction::watch_support::{retry_delay, write_snapshot_atomic};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const WS_URL: &str = "wss://stream.bybit.com/v5/public/spot";
const SNAPSHOT: &str = "prediction-bybit-spot-watch.json";
const SYMBOL: &str = "BTCUSDT";

#[derive(Debug, Clone, Serialize, PartialEq)]
struct BestQuote {
    bid: Option<String>,
    ask: Option<String>,
    spread: Option<String>,
    updated_at_ms: Option<i64>,
}

impl BestQuote {
    fn empty() -> Self {
        Self {
            bid: None,
            ask: None,
            spread: None,
            updated_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct LastTrade {
    price: String,
    qty: String,
    /// `BUY`/`SELL` tal como Bybit declara el lado del taker (`S`); no se infiere.
    side: String,
    traded_at_ms: i64,
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
    expires_at_ms: i64,
    mode: &'static str,
    execution: bool,
    persistence: &'static str,
    source: &'static str,
    symbol: &'static str,
    quote: BestQuote,
    last_trade: Option<LastTrade>,
    health: Health,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

/// El primer elemento de `b`/`a` en `orderbook.1` ya es el nivel top (Bybit lo ordena
/// mejor precio primero); no hace falta reconstruir profundidad para esta suscripción.
fn parse_orderbook_level1(data: &Value) -> Option<BestQuote> {
    let bid = data
        .get("b")
        .and_then(Value::as_array)
        .and_then(|levels| levels.first())
        .and_then(|level| level.as_array())
        .and_then(|level| level.first())
        .and_then(|price| text(Some(price)));
    let ask = data
        .get("a")
        .and_then(Value::as_array)
        .and_then(|levels| levels.first())
        .and_then(|level| level.as_array())
        .and_then(|level| level.first())
        .and_then(|price| text(Some(price)));
    if bid.is_none() && ask.is_none() {
        return None;
    }
    let spread = match (
        bid.as_deref().and_then(|v| v.parse::<f64>().ok()),
        ask.as_deref().and_then(|v| v.parse::<f64>().ok()),
    ) {
        (Some(bid), Some(ask)) => Some(format!("{:.2}", ask - bid)),
        _ => None,
    };
    Some(BestQuote {
        bid,
        ask,
        spread,
        updated_at_ms: Some(now_ms()),
    })
}

/// `publicTrade` entrega un array de trades por mensaje; se conserva el último (más
/// reciente) como último trade visto.
fn parse_last_public_trade(data: &Value) -> Option<LastTrade> {
    let trade = data.as_array()?.last()?;
    Some(LastTrade {
        price: text(trade.get("p"))?,
        qty: text(trade.get("v"))?,
        side: trade.get("S")?.as_str()?.to_uppercase(),
        traded_at_ms: trade.get("T")?.as_i64()?,
    })
}

/// Aplica un mensaje ya parseado según su `topic`. Devuelve `true` si cambió algo
/// publicable; los mensajes de control (ack de suscripción, pong) no tienen `topic` y
/// se ignoran sin error.
fn apply_event(quote: &mut BestQuote, last_trade: &mut Option<LastTrade>, message: &Value) -> bool {
    let Some(topic) = message.get("topic").and_then(Value::as_str) else {
        return false;
    };
    let Some(data) = message.get("data") else {
        return false;
    };
    match topic {
        "orderbook.1.BTCUSDT" => match parse_orderbook_level1(data) {
            Some(next) => {
                *quote = next;
                true
            }
            None => false,
        },
        "publicTrade.BTCUSDT" => match parse_last_public_trade(data) {
            Some(next) => {
                *last_trade = Some(next);
                true
            }
            None => false,
        },
        _ => false,
    }
}

fn subscription() -> String {
    serde_json::json!({
        "op": "subscribe",
        "args": ["orderbook.1.BTCUSDT", "publicTrade.BTCUSDT"],
    })
    .to_string()
}

fn snapshot(
    quote: BestQuote,
    last_trade: Option<LastTrade>,
    connected: bool,
    reconnects: u64,
    last_error: Option<String>,
) -> Snapshot {
    Snapshot {
        schema: "prediction-bybit-spot-watch-v1",
        generated_at_ms: now_ms(),
        expires_at_ms: now_ms() + 10_000,
        mode: "streaming-first",
        execution: false,
        persistence: "research-raw-optional",
        source: "bybit-spot",
        symbol: SYMBOL,
        quote,
        last_trade,
        health: Health {
            connected,
            reconnects,
            last_error,
        },
    }
}

struct Args {
    output: PathBuf,
    record_dir: Option<PathBuf>,
}

fn args() -> Args {
    let mut arguments = std::env::args().skip(1);
    let mut out = PathBuf::from("prediction-state");
    let mut record_dir = None;
    while let Some(argument) = arguments.next() {
        if argument == "--out" {
            if let Some(value) = arguments.next() {
                out = PathBuf::from(value);
            }
        } else if argument == "--record" {
            if let Some(value) = arguments.next() {
                record_dir = Some(PathBuf::from(value));
            }
        }
    }
    Args {
        output: out.join(SNAPSHOT),
        record_dir,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "prediction_bybit_spot_watch=info".into()),
        )
        .with_target(false)
        .init();

    let args = args();
    let output = args.output;
    let mut research = args
        .record_dir
        .as_deref()
        .map(|directory| ResearchWriter::open(directory, "bybit-spot", now_ms()))
        .transpose()?;
    if let Some(writer) = &research {
        info!(session = %writer.session_dir().display(), "captura RAW Bybit spot habilitada");
    }
    let mut reconnects = 0_u64;
    let mut last_error: Option<String> = None;
    info!("Bybit Spot Watch · {SYMBOL} · efímero/read-only");

    loop {
        let mut quote = BestQuote::empty();
        let mut last_trade: Option<LastTrade> = None;
        write_snapshot_atomic(
            &output,
            &snapshot(
                quote.clone(),
                last_trade.clone(),
                false,
                reconnects,
                last_error.clone(),
            ),
        )?;

        let connection =
            match tokio::time::timeout(Duration::from_secs(5), connect_async(WS_URL)).await {
                Ok(Ok(connection)) => connection,
                Ok(Err(error)) => {
                    last_error = Some(error.to_string());
                    reconnects += 1;
                    warn!("Bybit spot no conecta: {error}");
                    let delay = retry_delay(reconnects);
                    write_snapshot_atomic(
                        &output,
                        &snapshot(quote, last_trade, false, reconnects, last_error.clone()),
                    )?;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(_) => {
                    last_error = Some("conexión a Bybit expiró tras 5 segundos".to_string());
                    reconnects += 1;
                    warn!(
                        "Bybit spot no conecta: {}",
                        last_error.as_deref().unwrap_or_default()
                    );
                    let delay = retry_delay(reconnects);
                    write_snapshot_atomic(
                        &output,
                        &snapshot(quote, last_trade, false, reconnects, last_error.clone()),
                    )?;
                    tokio::time::sleep(delay).await;
                    continue;
                }
            };
        let (socket, _) = connection;
        let (mut writer, mut reader) = socket.split();
        if let Err(error) = writer.send(Message::Text(subscription().into())).await {
            last_error = Some(error.to_string());
            reconnects += 1;
            let delay = retry_delay(reconnects);
            write_snapshot_atomic(
                &output,
                &snapshot(quote, last_trade, false, reconnects, last_error.clone()),
            )?;
            tokio::time::sleep(delay).await;
            continue;
        }
        write_snapshot_atomic(
            &output,
            &snapshot(quote.clone(), last_trade.clone(), true, reconnects, None),
        )?;
        info!("Bybit spot conectado");
        // Bybit cierra canales públicos sin actividad; un ping cada 20s los mantiene vivos
        // con margen frente al límite documentado.
        let mut heartbeat = tokio::time::interval(Duration::from_secs(20));

        let disconnected = loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if let Err(error) = writer.send(Message::Text(r#"{"op":"ping"}"#.into())).await { break Some(error.to_string()); }
                }
                message = reader.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let received_at_ms = now_ms();
                        if let Some(writer) = research.as_mut() {
                            if let Err(error) = writer.record_text("bybit-spot-message", received_at_ms, &text) {
                                warn!("no se pudo registrar evento RAW: {error}");
                            }
                        }
                        if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                            if apply_event(&mut quote, &mut last_trade, &parsed) {
                                if let Err(error) = write_snapshot_atomic(&output, &snapshot(quote.clone(), last_trade.clone(), true, reconnects, None)) {
                                    warn!("no se pudo publicar snapshot efímero: {error}");
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => { if let Err(error) = writer.send(Message::Pong(bytes)).await { break Some(error.to_string()); } }
                    Some(Ok(Message::Close(_))) | None => break Some("Bybit cerró la conexión".to_string()),
                    Some(Err(error)) => break Some(error.to_string()),
                    _ => {}
                },
                _ = tokio::signal::ctrl_c() => {
                    if let Some(writer) = research.as_mut() { let _ = writer.flush(); }
                    let _ = std::fs::remove_file(&output);
                    info!("Bybit Spot Watch detenido; snapshot efímero eliminado");
                    return Ok(());
                }
            }
        };
        last_error = disconnected;
        reconnects += 1;
        warn!("Bybit spot desconectado; reintentando");
        let delay = retry_delay(reconnects);
        write_snapshot_atomic(
            &output,
            &snapshot(quote, last_trade, false, reconnects, last_error.clone()),
        )?;
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parsea_el_top_de_libro_y_calcula_spread() {
        let data = json!({"s": "BTCUSDT", "b": [["80977.4", "0.05"]], "a": [["80977.5", "0.40"]], "u": 1, "seq": 1});
        let quote = parse_orderbook_level1(&data).unwrap();
        assert_eq!(quote.bid.as_deref(), Some("80977.4"));
        assert_eq!(quote.ask.as_deref(), Some("80977.5"));
        assert_eq!(quote.spread.as_deref(), Some("0.10"));
    }

    #[test]
    fn conserva_el_lado_declarado_por_bybit_sin_inferirlo() {
        let data = json!([{"i": "1", "T": 1000_i64, "p": "80977.4", "v": "0.0005", "S": "Sell", "s": "BTCUSDT"}]);
        let trade = parse_last_public_trade(&data).unwrap();
        assert_eq!(trade.side, "SELL");
        assert_eq!(trade.price, "80977.4");
    }

    #[test]
    fn conserva_el_ultimo_trade_cuando_el_mensaje_trae_varios() {
        let data = json!([
            {"i": "1", "T": 1000_i64, "p": "1", "v": "1", "S": "Buy", "s": "BTCUSDT"},
            {"i": "2", "T": 2000_i64, "p": "2", "v": "2", "S": "Sell", "s": "BTCUSDT"}
        ]);
        let trade = parse_last_public_trade(&data).unwrap();
        assert_eq!(trade.traded_at_ms, 2000);
        assert_eq!(trade.side, "SELL");
    }

    #[test]
    fn ignora_mensajes_de_control_sin_topic() {
        let mut quote = BestQuote::empty();
        let mut last_trade = None;
        let ack = json!({"success": true, "ret_msg": "subscribe", "op": "subscribe"});
        assert!(!apply_event(&mut quote, &mut last_trade, &ack));
    }
}
