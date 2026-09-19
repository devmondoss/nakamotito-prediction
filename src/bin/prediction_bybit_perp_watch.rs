//! Bybit Perp Watch — BTC/USDT perpetuo lineal: top-of-book, trades, mark price y
//! funding, sin ejecución.
//!
//! Collector de la fase 3 (multi-venue) del plan cuantitativo, separado del spot
//! (`prediction_bybit_spot_watch`) porque basis/funding son información propia de
//! derivados. No sustituye ni reproduce el TWAP de Chainlink; es un insumo para
//! nowcasting y microestructura. Con `--record` conserva los payloads RAW para
//! investigación reproducible.

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

const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
const SNAPSHOT: &str = "prediction-bybit-perp-watch.json";
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

/// El canal `tickers` de Bybit manda un `snapshot` inicial y luego `delta`s que solo
/// traen los campos que cambiaron — se acumulan sobre el estado anterior, nunca se
/// reemplaza todo el struct con un delta parcial.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
struct Funding {
    mark_price: Option<String>,
    funding_rate: Option<String>,
    open_interest_value: Option<String>,
    updated_at_ms: Option<i64>,
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
    funding: Funding,
    health: Health,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

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

fn parse_last_public_trade(data: &Value) -> Option<LastTrade> {
    let trade = data.as_array()?.last()?;
    Some(LastTrade {
        price: text(trade.get("p"))?,
        qty: text(trade.get("v"))?,
        side: trade.get("S")?.as_str()?.to_uppercase(),
        traded_at_ms: trade.get("T")?.as_i64()?,
    })
}

/// Aplica un delta/snapshot de `tickers` sobre el estado acumulado. Un campo ausente en
/// este mensaje significa "sin cambio", no "vacío": nunca se pisa con `None`.
fn merge_ticker(funding: &mut Funding, data: &Value) -> bool {
    let mut changed = false;
    if let Some(value) = text(data.get("markPrice")) {
        funding.mark_price = Some(value);
        changed = true;
    }
    if let Some(value) = text(data.get("fundingRate")) {
        funding.funding_rate = Some(value);
        changed = true;
    }
    if let Some(value) = text(data.get("openInterestValue")) {
        funding.open_interest_value = Some(value);
        changed = true;
    }
    if changed {
        funding.updated_at_ms = Some(now_ms());
    }
    changed
}

fn apply_event(
    quote: &mut BestQuote,
    last_trade: &mut Option<LastTrade>,
    funding: &mut Funding,
    message: &Value,
) -> bool {
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
        "tickers.BTCUSDT" => merge_ticker(funding, data),
        _ => false,
    }
}

fn subscription() -> String {
    serde_json::json!({
        "op": "subscribe",
        "args": ["orderbook.1.BTCUSDT", "publicTrade.BTCUSDT", "tickers.BTCUSDT"],
    })
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn snapshot(
    quote: BestQuote,
    last_trade: Option<LastTrade>,
    funding: Funding,
    connected: bool,
    reconnects: u64,
    last_error: Option<String>,
) -> Snapshot {
    Snapshot {
        schema: "prediction-bybit-perp-watch-v1",
        generated_at_ms: now_ms(),
        expires_at_ms: now_ms() + 10_000,
        mode: "streaming-first",
        execution: false,
        persistence: "research-raw-optional",
        source: "bybit-perp",
        symbol: SYMBOL,
        quote,
        last_trade,
        funding,
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
                .unwrap_or_else(|_| "prediction_bybit_perp_watch=info".into()),
        )
        .with_target(false)
        .init();

    let args = args();
    let output = args.output;
    let mut research = args
        .record_dir
        .as_deref()
        .map(|directory| ResearchWriter::open(directory, "bybit-perp", now_ms()))
        .transpose()?;
    if let Some(writer) = &research {
        info!(session = %writer.session_dir().display(), "captura RAW Bybit perp habilitada");
    }
    let mut reconnects = 0_u64;
    let mut last_error: Option<String> = None;
    info!("Bybit Perp Watch · {SYMBOL} · efímero/read-only");

    loop {
        let mut quote = BestQuote::empty();
        let mut last_trade: Option<LastTrade> = None;
        let mut funding = Funding::default();
        write_snapshot_atomic(
            &output,
            &snapshot(
                quote.clone(),
                last_trade.clone(),
                funding.clone(),
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
                    warn!("Bybit perp no conecta: {error}");
                    let delay = retry_delay(reconnects);
                    write_snapshot_atomic(
                        &output,
                        &snapshot(
                            quote,
                            last_trade,
                            funding,
                            false,
                            reconnects,
                            last_error.clone(),
                        ),
                    )?;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(_) => {
                    last_error = Some("conexión a Bybit expiró tras 5 segundos".to_string());
                    reconnects += 1;
                    warn!(
                        "Bybit perp no conecta: {}",
                        last_error.as_deref().unwrap_or_default()
                    );
                    let delay = retry_delay(reconnects);
                    write_snapshot_atomic(
                        &output,
                        &snapshot(
                            quote,
                            last_trade,
                            funding,
                            false,
                            reconnects,
                            last_error.clone(),
                        ),
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
                &snapshot(
                    quote,
                    last_trade,
                    funding,
                    false,
                    reconnects,
                    last_error.clone(),
                ),
            )?;
            tokio::time::sleep(delay).await;
            continue;
        }
        write_snapshot_atomic(
            &output,
            &snapshot(
                quote.clone(),
                last_trade.clone(),
                funding.clone(),
                true,
                reconnects,
                None,
            ),
        )?;
        info!("Bybit perp conectado");
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
                            if let Err(error) = writer.record_text("bybit-perp-message", received_at_ms, &text) {
                                warn!("no se pudo registrar evento RAW: {error}");
                            }
                        }
                        if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                            if apply_event(&mut quote, &mut last_trade, &mut funding, &parsed) {
                                if let Err(error) = write_snapshot_atomic(&output, &snapshot(quote.clone(), last_trade.clone(), funding.clone(), true, reconnects, None)) {
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
                    info!("Bybit Perp Watch detenido; snapshot efímero eliminado");
                    return Ok(());
                }
            }
        };
        last_error = disconnected;
        reconnects += 1;
        warn!("Bybit perp desconectado; reintentando");
        let delay = retry_delay(reconnects);
        write_snapshot_atomic(
            &output,
            &snapshot(
                quote,
                last_trade,
                funding,
                false,
                reconnects,
                last_error.clone(),
            ),
        )?;
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parsea_el_top_de_libro() {
        let data = json!({"s": "BTCUSDT", "b": [["80934.3", "0.1"]], "a": [["80934.4", "4.0"]]});
        let quote = parse_orderbook_level1(&data).unwrap();
        assert_eq!(quote.bid.as_deref(), Some("80934.3"));
        assert_eq!(quote.spread.as_deref(), Some("0.10"));
    }

    #[test]
    fn el_ticker_acumula_campos_parciales_sin_pisar_los_no_declarados() {
        let mut funding = Funding::default();
        let first_delta = json!({"markPrice": "80940.44", "openInterestValue": "4621227726.88", "fundingRate": "0.00006983"});
        assert!(merge_ticker(&mut funding, &first_delta));
        assert_eq!(funding.mark_price.as_deref(), Some("80940.44"));

        // Un segundo delta solo trae bid/ask (irrelevantes para funding): no debe borrar
        // lo que ya se acumuló.
        let second_delta = json!({"bid1Price": "80934.30"});
        assert!(!merge_ticker(&mut funding, &second_delta));
        assert_eq!(funding.mark_price.as_deref(), Some("80940.44"));
        assert_eq!(funding.funding_rate.as_deref(), Some("0.00006983"));
    }

    #[test]
    fn aplica_eventos_por_topic() {
        let mut quote = BestQuote::empty();
        let mut last_trade = None;
        let mut funding = Funding::default();
        let ticker_message = json!({
            "topic": "tickers.BTCUSDT",
            "type": "delta",
            "data": {"symbol": "BTCUSDT", "markPrice": "1.0"},
        });
        assert!(apply_event(
            &mut quote,
            &mut last_trade,
            &mut funding,
            &ticker_message
        ));
        assert_eq!(funding.mark_price.as_deref(), Some("1.0"));
    }
}
