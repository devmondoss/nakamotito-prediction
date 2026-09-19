//! Binance Perp Watch — BTC/USDT-M top-of-book, trades, mark price y funding, sin ejecución.
//!
//! Collector de la fase 3 (multi-venue) del plan cuantitativo: formación de precio en el
//! perpetuo USD-M de Binance, separado del spot (`prediction_binance_spot_watch`) porque
//! basis/funding son información propia de derivados, no de spot. No sustituye ni
//! reproduce el TWAP de Chainlink; es un insumo para nowcasting y microestructura. Con
//! `--record` conserva los payloads RAW para investigación reproducible.

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

const WS_URL: &str = "wss://fstream.binance.com/stream?streams=btcusdt@bookTicker/btcusdt@aggTrade/btcusdt@markPrice@1s";
const SNAPSHOT: &str = "prediction-binance-perp-watch.json";
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
    /// Dirección del agresor: `BUY` si el trade se ejecutó contra el ask, `SELL` si
    /// contra el bid. Deriva del flag `m` (maker es comprador) de Binance.
    side: &'static str,
    traded_at_ms: i64,
}

/// `mark_price` es lo que usa Binance para liquidaciones y PnL, distinto del último
/// trade; `index_price` es el promedio de spot entre exchanges que Binance usa como
/// ancla del mark price. Ninguno de los dos es el TWAP de Chainlink ni lo sustituye.
#[derive(Debug, Clone, Serialize, PartialEq)]
struct Funding {
    mark_price: Option<String>,
    index_price: Option<String>,
    funding_rate: Option<String>,
    next_funding_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
}

impl Funding {
    fn empty() -> Self {
        Self {
            mark_price: None,
            index_price: None,
            funding_rate: None,
            next_funding_at_ms: None,
            updated_at_ms: None,
        }
    }
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

/// Los streams combinados de Binance envuelven cada mensaje como `{"stream": "...",
/// "data": {...}}`. El nombre del stream identifica el tipo de evento (bookTicker no
/// trae un campo `e` propio, a diferencia de aggTrade/markPriceUpdate).
fn stream_name(message: &Value) -> Option<&str> {
    message.get("stream")?.as_str()
}

fn parse_book_ticker(data: &Value) -> Option<BestQuote> {
    let bid = text(data.get("b"));
    let ask = text(data.get("a"));
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

fn parse_agg_trade(data: &Value) -> Option<LastTrade> {
    let price = text(data.get("p"))?;
    let qty = text(data.get("q"))?;
    let traded_at_ms = data.get("T")?.as_i64()?;
    let maker_is_buyer = data.get("m")?.as_bool()?;
    let side = if maker_is_buyer { "SELL" } else { "BUY" };
    Some(LastTrade {
        price,
        qty,
        side,
        traded_at_ms,
    })
}

fn parse_mark_price(data: &Value) -> Option<Funding> {
    Some(Funding {
        mark_price: text(data.get("p")),
        index_price: text(data.get("i")),
        funding_rate: text(data.get("r")),
        next_funding_at_ms: data.get("T").and_then(Value::as_i64),
        updated_at_ms: Some(now_ms()),
    })
}

/// Aplica un mensaje ya parseado del stream combinado. Devuelve `true` si cambió algo
/// publicable.
fn apply_event(
    quote: &mut BestQuote,
    last_trade: &mut Option<LastTrade>,
    funding: &mut Funding,
    message: &Value,
) -> bool {
    let Some(stream) = stream_name(message) else {
        return false;
    };
    let Some(data) = message.get("data") else {
        return false;
    };
    match stream {
        "btcusdt@bookTicker" => match parse_book_ticker(data) {
            Some(next) => {
                *quote = next;
                true
            }
            None => false,
        },
        "btcusdt@aggTrade" => match parse_agg_trade(data) {
            Some(next) => {
                *last_trade = Some(next);
                true
            }
            None => false,
        },
        "btcusdt@markPrice@1s" => match parse_mark_price(data) {
            Some(next) => {
                *funding = next;
                true
            }
            None => false,
        },
        _ => false,
    }
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
        schema: "prediction-binance-perp-watch-v1",
        generated_at_ms: now_ms(),
        expires_at_ms: now_ms() + 10_000,
        mode: "streaming-first",
        execution: false,
        persistence: "research-raw-optional",
        source: "binance-perp",
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
                .unwrap_or_else(|_| "prediction_binance_perp_watch=info".into()),
        )
        .with_target(false)
        .init();

    let args = args();
    let output = args.output;
    let mut research = args
        .record_dir
        .as_deref()
        .map(|directory| ResearchWriter::open(directory, "binance-perp", now_ms()))
        .transpose()?;
    if let Some(writer) = &research {
        info!(session = %writer.session_dir().display(), "captura RAW Binance perp habilitada");
    }
    let mut reconnects = 0_u64;
    let mut last_error: Option<String> = None;
    info!("Binance Perp Watch · {SYMBOL} · efímero/read-only");

    loop {
        let mut quote = BestQuote::empty();
        let mut last_trade: Option<LastTrade> = None;
        let mut funding = Funding::empty();
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
                    warn!("Binance perp no conecta: {error}");
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
                    last_error = Some("conexión a Binance expiró tras 5 segundos".to_string());
                    reconnects += 1;
                    warn!(
                        "Binance perp no conecta: {}",
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
        info!("Binance perp conectado");
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));

        let disconnected = loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if let Err(error) = writer.send(Message::Ping(Vec::new().into())).await {
                        break Some(error.to_string());
                    }
                }
                message = reader.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let received_at_ms = now_ms();
                        if let Some(writer) = research.as_mut() {
                            if let Err(error) = writer.record_text("binance-perp-message", received_at_ms, &text) {
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
                    Some(Ok(Message::Close(_))) | None => break Some("Binance cerró la conexión".to_string()),
                    Some(Err(error)) => break Some(error.to_string()),
                    _ => {}
                },
                _ = tokio::signal::ctrl_c() => {
                    if let Some(writer) = research.as_mut() { let _ = writer.flush(); }
                    let _ = std::fs::remove_file(&output);
                    info!("Binance Perp Watch detenido; snapshot efímero eliminado");
                    return Ok(());
                }
            }
        };
        last_error = disconnected;
        reconnects += 1;
        warn!("Binance perp desconectado; reintentando");
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
    fn parsea_el_book_ticker_y_calcula_spread() {
        let data = json!({"u": 1, "s": "BTCUSDT", "b": "80970.20", "B": "0.4", "a": "80970.30", "A": "22.3"});
        let quote = parse_book_ticker(&data).unwrap();
        assert_eq!(quote.bid.as_deref(), Some("80970.20"));
        assert_eq!(quote.spread.as_deref(), Some("0.10"));
    }

    #[test]
    fn deriva_el_lado_agresor_del_flag_maker_de_binance() {
        let taker_compra = json!({"p": "80970.3", "q": "0.01", "T": 1000_i64, "m": false});
        assert_eq!(parse_agg_trade(&taker_compra).unwrap().side, "BUY");
    }

    #[test]
    fn parsea_mark_price_y_funding() {
        let data = json!({
            "e": "markPriceUpdate", "s": "BTCUSDT",
            "p": "80995.10000000", "i": "80990.00000000",
            "r": "0.00010000", "T": 1789800000000_i64,
        });
        let funding = parse_mark_price(&data).unwrap();
        assert_eq!(funding.mark_price.as_deref(), Some("80995.10000000"));
        assert_eq!(funding.index_price.as_deref(), Some("80990.00000000"));
        assert_eq!(funding.funding_rate.as_deref(), Some("0.00010000"));
        assert_eq!(funding.next_funding_at_ms, Some(1789800000000));
    }

    #[test]
    fn aplica_eventos_del_stream_combinado_por_su_nombre() {
        let mut quote = BestQuote::empty();
        let mut last_trade = None;
        let mut funding = Funding::empty();
        let mark_message = json!({
            "stream": "btcusdt@markPrice@1s",
            "data": {"p": "1.0", "i": "1.0", "r": "0.0001", "T": 1_i64},
        });
        assert!(apply_event(
            &mut quote,
            &mut last_trade,
            &mut funding,
            &mark_message
        ));
        assert_eq!(funding.mark_price.as_deref(), Some("1.0"));
    }

    #[test]
    fn ignora_streams_desconocidos_sin_fallar() {
        let mut quote = BestQuote::empty();
        let mut last_trade = None;
        let mut funding = Funding::empty();
        let other = json!({"stream": "btcusdt@depth", "data": {}});
        assert!(!apply_event(
            &mut quote,
            &mut last_trade,
            &mut funding,
            &other
        ));
    }
}
