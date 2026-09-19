//! CLOB Watch — libro Up/Down de la ronda BTC 5m vigente, sin ejecución.
//!
//! Descubre la ronda actual por Gamma y se suscribe al Market Channel público de Polymarket.
//! El resultado es un snapshot IPC efímero para la consola. Con `--record` conserva los payloads
//! RAW de Gamma y CLOB para investigación reproducible.

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use nakamotito_prediction::research::{now_ms, ResearchWriter};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const GAMMA: &str = "https://gamma-api.polymarket.com/markets?slug=";
const CLOB_WSS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const SNAPSHOT: &str = "prediction-clob-watch.json";

#[derive(Clone, Debug, Serialize, PartialEq)]
struct Market {
    slug: String,
    question: String,
    end_date: Option<String>,
    settlement: SettlementRule,
    outcomes: Vec<String>,
    token_ids: Vec<String>,
}

/// Metadatos de resolución extraídos de Gamma por ronda. Se registran junto al payload RAW para
/// que el replay no dependa de una regla global ni de memoria humana.
#[derive(Clone, Debug, Serialize, PartialEq)]
struct SettlementRule {
    resolution_source: Option<String>,
    start_at: Option<String>,
    end_at: Option<String>,
    twap_enabled: Option<bool>,
    twap_lookback_seconds: Option<u64>,
    fee_type: Option<String>,
    fees_enabled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct OutcomeBook {
    outcome: String,
    token_id: String,
    best_bid: Option<String>,
    best_ask: Option<String>,
    spread: Option<String>,
    event_at_ms: Option<i64>,
    received_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct Health {
    connected: bool,
    reconnects: u64,
    last_error: Option<String>,
    stage: &'static str,
    next_retry_in_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct Snapshot {
    schema: &'static str,
    generated_at_ms: i64,
    expires_at_ms: i64,
    mode: &'static str,
    execution: bool,
    persistence: &'static str,
    market: Option<Market>,
    books: Vec<OutcomeBook>,
    health: Health,
}

fn current_slug() -> String {
    let round_start_s = now_ms() / 1_000 / 300 * 300;
    format!("btc-updown-5m-{round_start_s}")
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn timestamp(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn string_array(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        Value::String(encoded) => serde_json::from_str::<Vec<String>>(encoded).ok(),
        _ => None,
    }
}

fn parse_market(value: &Value) -> Option<Market> {
    let market = value.as_array()?.first()?;
    let outcomes = string_array(market.get("outcomes")?)?;
    let token_ids = string_array(market.get("clobTokenIds")?)?;
    if outcomes.len() != 2
        || outcomes.len() != token_ids.len()
        || !outcomes.iter().any(|x| x == "Up")
        || !outcomes.iter().any(|x| x == "Down")
    {
        return None;
    }
    Some(Market {
        slug: market.get("slug")?.as_str()?.to_owned(),
        question: market.get("question")?.as_str()?.to_owned(),
        end_date: market
            .get("endDate")
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        settlement: SettlementRule {
            resolution_source: market
                .get("resolutionSource")
                .and_then(Value::as_str)
                .map(str::to_owned),
            start_at: market
                .get("eventStartTime")
                .or_else(|| market.get("startTime"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            end_at: market
                .get("endDate")
                .and_then(Value::as_str)
                .map(str::to_owned),
            twap_enabled: market
                .get("cryptoMarketConfig")
                .and_then(|value| value.get("twapEnabled"))
                .and_then(Value::as_bool),
            twap_lookback_seconds: market
                .get("cryptoMarketConfig")
                .and_then(|value| value.get("twapLookbackSeconds"))
                .and_then(Value::as_u64),
            fee_type: market
                .get("feeType")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fees_enabled: market.get("feesEnabled").and_then(Value::as_bool),
        },
        outcomes,
        token_ids,
    })
}

async fn discover(
    client: &reqwest::Client,
    research: &mut Option<ResearchWriter>,
) -> Result<Market> {
    let slug = current_slug();
    let response = client
        .get(format!("{GAMMA}{slug}"))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    if let Some(writer) = research.as_mut() {
        writer.record_value("gamma-market-response", now_ms(), &response)?;
    }
    parse_market(&response)
        .with_context(|| format!("Gamma no devolvió ronda BTC Up/Down válida para {slug}"))
}

fn empty_books(market: &Market) -> Vec<OutcomeBook> {
    market
        .outcomes
        .iter()
        .zip(&market.token_ids)
        .map(|(outcome, token_id)| OutcomeBook {
            outcome: outcome.clone(),
            token_id: token_id.clone(),
            best_bid: None,
            best_ask: None,
            spread: None,
            event_at_ms: None,
            received_at_ms: None,
        })
        .collect()
}

fn extrema(levels: Option<&Vec<Value>>, bid: bool) -> Option<String> {
    let mut candidate: Option<(f64, String)> = None;
    for level in levels? {
        let price = text(level.get("price"))?;
        let numeric = price.parse::<f64>().ok()?;
        match &candidate {
            None => candidate = Some((numeric, price)),
            Some((current, _)) if (bid && numeric > *current) || (!bid && numeric < *current) => {
                candidate = Some((numeric, price))
            }
            _ => {}
        }
    }
    candidate.map(|(_, value)| value)
}

fn apply_event(books: &mut [OutcomeBook], text_message: &str, received_at_ms: i64) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(text_message) else {
        return false;
    };
    let event_type = message.get("event_type").and_then(Value::as_str);
    let event_at_ms = timestamp(message.get("timestamp"));

    if event_type == Some("book") {
        let Some(asset) = message.get("asset_id").and_then(Value::as_str) else {
            return false;
        };
        let Some(book) = books.iter_mut().find(|book| book.token_id == asset) else {
            return false;
        };
        book.best_bid = extrema(message.get("bids").and_then(Value::as_array), true);
        book.best_ask = extrema(message.get("asks").and_then(Value::as_array), false);
        book.spread = match (&book.best_bid, &book.best_ask) {
            (Some(bid), Some(ask)) => match (bid.parse::<f64>(), ask.parse::<f64>()) {
                (Ok(bid), Ok(ask)) => Some(format!("{:.6}", ask - bid)),
                _ => None,
            },
            _ => None,
        };
        book.event_at_ms = event_at_ms;
        book.received_at_ms = Some(received_at_ms);
        return true;
    }

    if event_type == Some("best_bid_ask") {
        let Some(asset) = message.get("asset_id").and_then(Value::as_str) else {
            return false;
        };
        let Some(book) = books.iter_mut().find(|book| book.token_id == asset) else {
            return false;
        };
        book.best_bid = text(message.get("best_bid"));
        book.best_ask = text(message.get("best_ask"));
        book.spread = text(message.get("spread"));
        book.event_at_ms = event_at_ms;
        book.received_at_ms = Some(received_at_ms);
        return true;
    }
    false
}

fn subscription(market: &Market) -> String {
    serde_json::json!({
        "assets_ids": market.token_ids,
        "type": "market",
        "custom_feature_enabled": true,
    })
    .to_string()
}

fn write_snapshot(
    path: &Path,
    market: Option<Market>,
    books: &[OutcomeBook],
    connected: bool,
    reconnects: u64,
    last_error: Option<String>,
    stage: &'static str,
    next_retry_in_ms: Option<u64>,
) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("snapshot sin directorio padre")?)?;
    let now = now_ms();
    let snapshot = Snapshot {
        schema: "prediction-clob-watch-v1",
        generated_at_ms: now,
        expires_at_ms: now + 10_000,
        mode: "streaming-first",
        execution: false,
        persistence: "none",
        market,
        books: books.to_vec(),
        health: Health {
            connected,
            reconnects,
            last_error,
            stage,
            next_retry_in_ms,
        },
    };
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&snapshot)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// Por qué terminó el bucle de streaming de una ronda. Un `Rollover` es un evento
/// esperado cada 5 minutos y se maneja distinto de un `Error` real: no debe acumular
/// reintentos ni esperar el backoff pensado para fallas de red.
enum StreamExit {
    Rollover,
    Error(String),
}

/// Exponential backoff limits retry pressure while a remote endpoint or the route to it is down.
/// It is intentionally bounded: recovery remains prompt when the service returns.
fn retry_delay(reconnects: u64) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(reconnects.min(4) as u32).min(30))
}

async fn wait_retry(
    output: &Path,
    market: Option<Market>,
    books: &[OutcomeBook],
    reconnects: u64,
    error: String,
    stage: &'static str,
) -> Result<()> {
    let delay = retry_delay(reconnects);
    write_snapshot(
        output,
        market,
        books,
        false,
        reconnects,
        Some(error),
        stage,
        Some(delay.as_millis() as u64),
    )?;
    tokio::time::sleep(delay).await;
    Ok(())
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
                .unwrap_or_else(|_| "prediction_clob_watch=info".into()),
        )
        .with_target(false)
        .init();

    // Never let a network route hold the watcher for the operating system's full TCP timeout
    // (often ~60 seconds). A failed feed must become visible and retry promptly.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(7))
        .build()?;
    let args = args();
    let output = args.output;
    let mut research = args
        .record_dir
        .as_deref()
        .map(|directory| ResearchWriter::open(directory, "polymarket-gamma-clob", now_ms()))
        .transpose()?;
    if let Some(writer) = &research {
        info!(session = %writer.session_dir().display(), "captura RAW CLOB habilitada");
    }
    let mut reconnects = 0_u64;
    let mut last_error = None;
    info!("CLOB Watch · BTC Up/Down 5m · efímero/read-only");

    loop {
        let market = match discover(&client, &mut research).await {
            Ok(market) => market,
            Err(error) => {
                last_error = Some(error.to_string());
                warn!("no se pudo descubrir la ronda: {error}");
                reconnects += 1;
                wait_retry(
                    &output,
                    None,
                    &[],
                    reconnects,
                    last_error.clone().unwrap_or_default(),
                    "gamma-discovery",
                )
                .await?;
                continue;
            }
        };
        let mut books = empty_books(&market);
        write_snapshot(
            &output,
            Some(market.clone()),
            &books,
            false,
            reconnects,
            last_error.clone(),
            "clob-connecting",
            None,
        )?;

        let connection =
            match tokio::time::timeout(Duration::from_secs(5), connect_async(CLOB_WSS)).await {
                Ok(Ok(connection)) => connection,
                Ok(Err(error)) => {
                    last_error = Some(error.to_string());
                    reconnects += 1;
                    warn!("CLOB no conecta: {error}");
                    wait_retry(
                        &output,
                        Some(market),
                        &books,
                        reconnects,
                        last_error.clone().unwrap_or_default(),
                        "clob-connect",
                    )
                    .await?;
                    continue;
                }
                Err(_) => {
                    let error = anyhow!("CLOB connection timed out after 5 seconds");
                    last_error = Some(error.to_string());
                    reconnects += 1;
                    warn!("CLOB no conecta: {error}");
                    wait_retry(
                        &output,
                        Some(market),
                        &books,
                        reconnects,
                        last_error.clone().unwrap_or_default(),
                        "clob-connect",
                    )
                    .await?;
                    continue;
                }
            };
        let (socket, _) = connection;
        let (mut writer, mut reader) = socket.split();
        if let Err(error) = writer
            .send(Message::Text(subscription(&market).into()))
            .await
        {
            last_error = Some(error.to_string());
            reconnects += 1;
            wait_retry(
                &output,
                Some(market),
                &books,
                reconnects,
                last_error.clone().unwrap_or_default(),
                "clob-subscribe",
            )
            .await?;
            continue;
        }
        write_snapshot(
            &output,
            Some(market.clone()),
            &books,
            true,
            reconnects,
            None,
            "streaming",
            None,
        )?;
        info!("CLOB conectado · {}", market.slug);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        // Comprueba el límite de ronda con margen de un segundo: el cambio de ronda es un
        // evento esperado cada 5 minutos, no una falla, así que no debe demorar como una.
        let mut rollover = tokio::time::interval(Duration::from_secs(1));

        let why = loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if let Err(error) = writer.send(Message::Text("PING".into())).await { break StreamExit::Error(error.to_string()); }
                }
                _ = rollover.tick() => {
                    if current_slug() != market.slug { break StreamExit::Rollover; }
                }
                message = reader.next() => match message {
                    Some(Ok(Message::Text(message))) => {
                        let received_at_ms = now_ms();
                        if let Some(writer) = research.as_mut() {
                            if let Err(error) = writer.record_text("clob-message", received_at_ms, &message) {
                                warn!("no se pudo registrar evento RAW: {error}");
                            }
                        }
                        if apply_event(&mut books, &message, received_at_ms) {
                            if let Err(error) = write_snapshot(&output, Some(market.clone()), &books, true, reconnects, None, "streaming", None) {
                                warn!("no se pudo publicar libro efímero: {error}");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => { if let Err(error) = writer.send(Message::Pong(bytes)).await { break StreamExit::Error(error.to_string()); } }
                    Some(Ok(Message::Close(_))) | None => break StreamExit::Error("CLOB cerró la conexión".to_string()),
                    Some(Err(error)) => break StreamExit::Error(error.to_string()),
                    _ => {}
                },
                _ = tokio::signal::ctrl_c() => {
                    if let Some(writer) = research.as_mut() { let _ = writer.flush(); }
                    let _ = std::fs::remove_file(&output);
                    info!("CLOB Watch detenido; snapshot efímero eliminado");
                    return Ok(());
                }
            }
        };
        match why {
            // Una ronda nueva no es un error: no acumula `reconnects` (esa cuenta es
            // información de salud real) ni espera el backoff exponencial pensado para
            // fallas de red. Se redescubre de inmediato en la siguiente vuelta del bucle.
            StreamExit::Rollover => {
                info!("nueva ronda BTC 5m detectada; redescubriendo sin backoff");
            }
            StreamExit::Error(error) => {
                last_error = Some(error);
                reconnects += 1;
                wait_retry(
                    &output,
                    Some(market),
                    &books,
                    reconnects,
                    last_error.clone().unwrap_or_default(),
                    "clob-stream",
                )
                .await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreta_los_tokens_y_outcomes_de_gamma() {
        let response = serde_json::json!([{"slug":"btc-updown-5m-1","question":"Bitcoin Up or Down","endDate":"x","eventStartTime":"s","resolutionSource":"chainlink","cryptoMarketConfig":{"twapEnabled":true,"twapLookbackSeconds":60},"feeType":"crypto","feesEnabled":true,"outcomes":"[\"Up\",\"Down\"]","clobTokenIds":"[\"up\",\"down\"]"}]);
        assert_eq!(
            parse_market(&response),
            Some(Market {
                slug: "btc-updown-5m-1".into(),
                question: "Bitcoin Up or Down".into(),
                end_date: Some("x".into()),
                settlement: SettlementRule {
                    resolution_source: Some("chainlink".into()),
                    start_at: Some("s".into()),
                    end_at: Some("x".into()),
                    twap_enabled: Some(true),
                    twap_lookback_seconds: Some(60),
                    fee_type: Some("crypto".into()),
                    fees_enabled: Some(true)
                },
                outcomes: vec!["Up".into(), "Down".into()],
                token_ids: vec!["up".into(), "down".into()]
            })
        );
    }

    #[test]
    fn extrae_el_mejor_bid_y_ask_del_snapshot() {
        let mut books = vec![OutcomeBook {
            outcome: "Up".into(),
            token_id: "up".into(),
            best_bid: None,
            best_ask: None,
            spread: None,
            event_at_ms: None,
            received_at_ms: None,
        }];
        let event = r#"{"event_type":"book","asset_id":"up","bids":[{"price":"0.42"},{"price":"0.51"}],"asks":[{"price":"0.61"},{"price":"0.55"}],"timestamp":"100"}"#;
        assert!(apply_event(&mut books, event, 101));
        assert_eq!(books[0].best_bid.as_deref(), Some("0.51"));
        assert_eq!(books[0].best_ask.as_deref(), Some("0.55"));
        assert_eq!(books[0].spread.as_deref(), Some("0.040000"));
    }

    #[test]
    fn backoff_es_acotado() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
        assert_eq!(retry_delay(9), Duration::from_secs(16));
    }
}
