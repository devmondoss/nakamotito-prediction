//! Reconstrucción determinista del libro L2 del CLOB de Polymarket a partir del RAW capturado.
//!
//! No abre red: opera solo sobre mensajes `book` y `price_change` ya capturados por
//! `prediction_clob_watch --record`. Polymarket no documenta un número de secuencia ni el
//! algoritmo de su campo `hash` (confirmado contra la documentación oficial el 2026-09-18).
//!
//! La primera versión de este reconstructor intentó detectar huecos comparando el mejor bid/ask
//! reconstruido localmente contra el que cada `price_change` declara. Validado contra las tres
//! sesiones reales capturadas el 2026-09-18, esa señal resultó **no confiable**: durante ráfagas
//! de alta frecuencia (cientos de mensajes por segundo en un movimiento rápido) aparecen cadenas
//! largas de discrepancias transitorias — no solo el patrón de dos mensajes de un trade partido en
//! niveles — y como el estado quedaba pegado hasta el siguiente `book` (~2 % de los mensajes), una
//! sola ráfaga contaminaba miles de updates posteriores sin que hubiera un hueco real. Esa
//! comparación queda conservada como campo informativo (`declared_best_mismatch`), no sticky y sin
//! efecto en `quality`, para investigación futura del comportamiento exacto de Polymarket.
//!
//! La calidad del libro se basa en dos señales que sí son verificables sin depender de lo que el
//! proveedor declara: (1) un libro que cruza (`best_bid >= best_ask` en nuestra propia
//! reconstrucción) es imposible en un mercado sano y siempre indica una inconsistencia real; (2) el
//! silencio prolongado en un asset activo es visible con solo mirar los relojes locales.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq)]
struct Level {
    price: String,
    size: String,
}

#[derive(Debug, Default)]
struct AssetBook {
    bids: BTreeMap<i64, Level>,
    asks: BTreeMap<i64, Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Buy,
    Sell,
}

fn parse_side(value: &str) -> Option<Side> {
    match value {
        "BUY" => Some(Side::Buy),
        "SELL" => Some(Side::Sell),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Escala a entero fijo para ordenar sin depender de comparaciones de punto flotante. Los tick
/// sizes observados llegan hasta 0.001; 1e6 deja margen sin perder precisión práctica.
fn price_key(price: &str) -> Option<i64> {
    let value: f64 = price.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some((value * 1_000_000.0).round() as i64)
}

fn is_zero(size: &str) -> bool {
    size.parse::<f64>()
        .map(|value| value == 0.0)
        .unwrap_or(false)
}

impl AssetBook {
    fn load_side(side: &mut BTreeMap<i64, Level>, levels: &[Value]) {
        side.clear();
        for level in levels {
            let (Some(price), Some(size)) = (text(level.get("price")), text(level.get("size")))
            else {
                continue;
            };
            if is_zero(&size) {
                continue;
            }
            if let Some(key) = price_key(&price) {
                side.insert(key, Level { price, size });
            }
        }
    }

    fn apply_book(&mut self, bids: &[Value], asks: &[Value]) {
        Self::load_side(&mut self.bids, bids);
        Self::load_side(&mut self.asks, asks);
    }

    fn apply_level(&mut self, side: Side, price: &str, size: &str) {
        let Some(key) = price_key(price) else {
            return;
        };
        let map = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        if is_zero(size) {
            map.remove(&key);
        } else {
            map.insert(
                key,
                Level {
                    price: price.to_owned(),
                    size: size.to_owned(),
                },
            );
        }
    }

    /// Precio más alto entre las posturas de compra.
    fn best_bid(&self) -> Option<&Level> {
        self.bids.values().next_back()
    }

    /// Precio más bajo entre las posturas de venta.
    fn best_ask(&self) -> Option<&Level> {
        self.asks.values().next()
    }

    /// Un libro sano nunca tiene el mejor bid al mismo precio o por encima del mejor ask. Un lado
    /// vacío no cuenta como cruzado: un libro fino de un solo lado es normal cerca de settlement.
    fn is_crossed(&self) -> bool {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => match (bid.price.parse::<f64>(), ask.price.parse::<f64>()) {
                (Ok(bid), Ok(ask)) => bid >= ask,
                _ => false,
            },
            _ => false,
        }
    }
}

/// Contraste puramente informativo contra el `best_bid`/`best_ask` que el proveedor declaró en el
/// mismo mensaje. No determina `quality` (ver comentario del módulo); sirve para investigar el
/// patrón de batching de Polymarket, no como veredicto de integridad.
fn declared_mismatch(local: Option<&Level>, declared: Option<&str>) -> Option<bool> {
    match (local, declared) {
        (Some(local), Some(declared)) => {
            match (local.price.parse::<f64>(), declared.parse::<f64>()) {
                (Ok(local), Ok(declared)) => Some((local - declared).abs() > 1e-9),
                _ => None,
            }
        }
        (None, None) => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum L2Quality {
    /// Aún no llegó un `book` completo para este asset en esta reconstrucción.
    AwaitingSnapshot,
    /// Último `book` aplicado y el libro reconstruido no cruza. No implica que coincida byte a
    /// byte con el estado real del proveedor —eso no es verificable con lo que Polymarket expone—,
    /// solo que es internamente consistente.
    Synced,
    /// Este update llegó tras un silencio mayor al esperado; no implica libro corrupto, solo
    /// baja actividad — se limpia solo con el siguiente evento.
    Stale,
    /// El libro reconstruido tiene `best_bid >= best_ask`: inconsistencia real e imposible en un
    /// mercado sano. Persiste hasta el siguiente `book`, que es el único resync de confianza.
    Crossed,
}

impl L2Quality {
    fn as_str(self) -> &'static str {
        match self {
            L2Quality::AwaitingSnapshot => "AWAITING_SNAPSHOT",
            L2Quality::Synced => "SYNCED",
            L2Quality::Stale => "STALE",
            L2Quality::Crossed => "CROSSED",
        }
    }
}

struct AssetState {
    book: AssetBook,
    quality: L2Quality,
    quality_reason: Option<&'static str>,
    last_event_at_ms: Option<i64>,
}

impl AssetState {
    fn new() -> Self {
        Self {
            book: AssetBook::default(),
            quality: L2Quality::AwaitingSnapshot,
            quality_reason: Some("no_snapshot_yet"),
            last_event_at_ms: None,
        }
    }
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct L2Update {
    pub schema: &'static str,
    pub asset_id: String,
    pub event_at_ms: Option<i64>,
    pub received_at_ms: i64,
    pub best_bid: Option<String>,
    pub best_ask: Option<String>,
    pub spread: Option<String>,
    pub bid_levels: usize,
    pub ask_levels: usize,
    pub quality: &'static str,
    pub quality_reason: Option<&'static str>,
    /// `None` cuando no aplica (evento `book`, sin libro previo, o falta el campo declarado en el
    /// mensaje); `Some(true/false)` es un contraste observacional, no un veredicto de integridad.
    /// Ver el comentario del módulo.
    pub declared_best_mismatch: Option<bool>,
}

/// Reconstructor con estado por `asset_id`. Una instancia cubre una sesión de replay; no persiste
/// entre sesiones ni asume relación entre asset_ids de rondas distintas.
pub struct L2Reconstructor {
    assets: HashMap<String, AssetState>,
    stale_after_ms: i64,
}

impl L2Reconstructor {
    pub fn new(stale_after_ms: i64) -> Self {
        Self {
            assets: HashMap::new(),
            stale_after_ms,
        }
    }

    /// Sin un `book` de base, la profundidad acumulada de deltas sueltos no es una afirmación
    /// defendible: se reporta ausente (`None`/`0`) en vez de una mezcla parcial no verificada.
    fn summarize(
        &self,
        asset_id: &str,
        event_at_ms: Option<i64>,
        received_at_ms: i64,
        declared_best_mismatch: Option<bool>,
    ) -> L2Update {
        let state = self.assets.get(asset_id).expect("state recién insertado");
        if state.quality == L2Quality::AwaitingSnapshot {
            return L2Update {
                schema: "prediction-clob-l2-update-v1",
                asset_id: asset_id.to_owned(),
                event_at_ms,
                received_at_ms,
                best_bid: None,
                best_ask: None,
                spread: None,
                bid_levels: 0,
                ask_levels: 0,
                quality: state.quality.as_str(),
                quality_reason: state.quality_reason,
                declared_best_mismatch: None,
            };
        }
        let best_bid = state.book.best_bid();
        let best_ask = state.book.best_ask();
        let spread = match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => match (bid.price.parse::<f64>(), ask.price.parse::<f64>()) {
                (Ok(bid), Ok(ask)) => Some(format!("{:.6}", ask - bid)),
                _ => None,
            },
            _ => None,
        };
        L2Update {
            schema: "prediction-clob-l2-update-v1",
            asset_id: asset_id.to_owned(),
            event_at_ms,
            received_at_ms,
            best_bid: best_bid.map(|level| level.price.clone()),
            best_ask: best_ask.map(|level| level.price.clone()),
            spread,
            bid_levels: state.book.bids.len(),
            ask_levels: state.book.asks.len(),
            quality: state.quality.as_str(),
            quality_reason: state.quality_reason,
            declared_best_mismatch,
        }
    }

    fn apply_book(
        &mut self,
        asset_id: &str,
        bids: &[Value],
        asks: &[Value],
        event_at_ms: Option<i64>,
        received_at_ms: i64,
    ) -> L2Update {
        let state = self
            .assets
            .entry(asset_id.to_owned())
            .or_insert_with(AssetState::new);
        state.book.apply_book(bids, asks);
        state.quality = if state.book.is_crossed() {
            L2Quality::Crossed
        } else {
            L2Quality::Synced
        };
        state.quality_reason = match state.quality {
            L2Quality::Crossed => Some("crossed_book"),
            _ => None,
        };
        state.last_event_at_ms = Some(received_at_ms);
        self.summarize(asset_id, event_at_ms, received_at_ms, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_price_change(
        &mut self,
        asset_id: &str,
        side: Side,
        price: &str,
        size: &str,
        declared_best_bid: Option<&str>,
        declared_best_ask: Option<&str>,
        event_at_ms: Option<i64>,
        received_at_ms: i64,
    ) -> L2Update {
        let state = self
            .assets
            .entry(asset_id.to_owned())
            .or_insert_with(AssetState::new);
        let idle_ms_before = state.last_event_at_ms.map(|last| received_at_ms - last);
        let was_awaiting = state.quality == L2Quality::AwaitingSnapshot;
        let was_crossed = state.quality == L2Quality::Crossed;

        state.book.apply_level(side, price, size);
        state.last_event_at_ms = Some(received_at_ms);

        let mismatch_bid = declared_mismatch(state.book.best_bid(), declared_best_bid);
        let mismatch_ask = declared_mismatch(state.book.best_ask(), declared_best_ask);
        let declared_best_mismatch = match (mismatch_bid, mismatch_ask) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(false) || b.unwrap_or(false)),
        };

        if was_awaiting {
            // La profundidad sigue incompleta sin un `book` de base; no se finge sincronía.
            state.quality = L2Quality::AwaitingSnapshot;
            state.quality_reason = Some("no_snapshot_yet");
        } else if was_crossed || state.book.is_crossed() {
            state.quality = L2Quality::Crossed;
            state.quality_reason = Some("crossed_book");
        } else if idle_ms_before.is_some_and(|idle| idle > self.stale_after_ms) {
            state.quality = L2Quality::Stale;
            state.quality_reason = Some("idle_before_update");
        } else {
            state.quality = L2Quality::Synced;
            state.quality_reason = None;
        }
        self.summarize(
            asset_id,
            event_at_ms,
            received_at_ms,
            if was_awaiting {
                None
            } else {
                declared_best_mismatch
            },
        )
    }

    /// Aplica un mensaje RAW del Market Channel (el payload ya deserializado, sin el sobre de
    /// captura). Devuelve un update por cada asset_id que el mensaje haya tocado; un
    /// `price_change` puede tocar varios (p. ej. Up y Down de la misma ronda a la vez).
    pub fn apply_clob_message(
        &mut self,
        message: &Value,
        event_at_ms: Option<i64>,
        received_at_ms: i64,
    ) -> Vec<L2Update> {
        match message.get("event_type").and_then(Value::as_str) {
            Some("book") => {
                let Some(asset_id) = message.get("asset_id").and_then(Value::as_str) else {
                    return Vec::new();
                };
                let empty = Vec::new();
                let bids = message
                    .get("bids")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty);
                let asks = message
                    .get("asks")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty);
                vec![self.apply_book(asset_id, bids, asks, event_at_ms, received_at_ms)]
            }
            Some("price_change") => {
                let Some(changes) = message.get("price_changes").and_then(Value::as_array) else {
                    return Vec::new();
                };
                changes
                    .iter()
                    .filter_map(|change| {
                        let asset_id = change.get("asset_id").and_then(Value::as_str)?;
                        let side = parse_side(change.get("side").and_then(Value::as_str)?)?;
                        let price = text(change.get("price"))?;
                        let size = text(change.get("size"))?;
                        let declared_best_bid = text(change.get("best_bid"));
                        let declared_best_ask = text(change.get("best_ask"));
                        Some(self.apply_price_change(
                            asset_id,
                            side,
                            &price,
                            &size,
                            declared_best_bid.as_deref(),
                            declared_best_ask.as_deref(),
                            event_at_ms,
                            received_at_ms,
                        ))
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn espera_snapshot_antes_de_confiar_en_la_profundidad() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        let message = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.5", "size": "10", "best_bid": "0.5", "best_ask": "0.6"}
            ],
        });
        let updates = reconstructor.apply_clob_message(&message, Some(1), 1);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].quality, "AWAITING_SNAPSHOT");
        assert_eq!(
            updates[0].best_bid, None,
            "sin book previo no se afirma profundidad"
        );
    }

    #[test]
    fn aplica_book_y_luego_deltas_de_price_change() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        let book = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.40", "size": "100"}, {"price": "0.45", "size": "50"}],
            "asks": [{"price": "0.60", "size": "80"}, {"price": "0.55", "size": "40"}],
            "timestamp": "1000",
        });
        let after_book = reconstructor.apply_clob_message(&book, Some(1000), 1001);
        assert_eq!(after_book[0].quality, "SYNCED");
        assert_eq!(after_book[0].best_bid.as_deref(), Some("0.45"));
        assert_eq!(after_book[0].best_ask.as_deref(), Some("0.55"));
        assert_eq!(after_book[0].bid_levels, 2);

        // Sube una nueva mejor postura de compra a 0.50; el reconstruido debe reflejarla.
        let delta = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.50", "size": "20", "best_bid": "0.50", "best_ask": "0.55"}
            ],
        });
        let after_delta = reconstructor.apply_clob_message(&delta, Some(1500), 1501);
        assert_eq!(after_delta[0].quality, "SYNCED");
        assert_eq!(after_delta[0].best_bid.as_deref(), Some("0.50"));
        assert_eq!(after_delta[0].bid_levels, 3);
        assert_eq!(after_delta[0].declared_best_mismatch, Some(false));
    }

    #[test]
    fn size_cero_elimina_el_nivel() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        let book = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.40", "size": "100"}, {"price": "0.45", "size": "50"}],
            "asks": [],
            "timestamp": "1000",
        });
        reconstructor.apply_clob_message(&book, Some(1000), 1001);
        let delta = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.45", "size": "0", "best_bid": "0.40", "best_ask": null}
            ],
        });
        let after = reconstructor.apply_clob_message(&delta, Some(1200), 1201);
        assert_eq!(after[0].best_bid.as_deref(), Some("0.40"));
        assert_eq!(after[0].bid_levels, 1);
    }

    #[test]
    fn un_trade_partido_en_dos_mensajes_no_degrada_la_calidad() {
        // Patrón real observado el 2026-09-18: un trade que vacía un nivel de venta y agrega uno
        // de compra llega como dos mensajes con el mismo timestamp; el primero declara el mejor
        // precio *final* de la operación completa, que nuestro libro aún no refleja a mitad de
        // aplicar. `declared_best_mismatch` lo registra como observación, pero `quality` se
        // mantiene SYNCED porque el libro reconstruido nunca cruza.
        let mut reconstructor = L2Reconstructor::new(30_000);
        let book = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.73", "size": "100"}],
            "asks": [{"price": "0.74", "size": "10"}, {"price": "0.75", "size": "100"}],
            "timestamp": "1000",
        });
        reconstructor.apply_clob_message(&book, Some(1000), 1001);

        let first_of_pair = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.71", "size": "1777.42", "best_bid": "0.73", "best_ask": "0.75"}
            ],
        });
        let after_first = reconstructor.apply_clob_message(&first_of_pair, Some(1200), 1201);
        assert_eq!(after_first[0].quality, "SYNCED");
        assert_eq!(
            after_first[0].declared_best_mismatch,
            Some(true),
            "se observa la discrepancia transitoria aunque no degrade la calidad"
        );

        let second_of_pair = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "SELL", "price": "0.74", "size": "0", "best_bid": "0.73", "best_ask": "0.75"}
            ],
        });
        let after_second = reconstructor.apply_clob_message(&second_of_pair, Some(1200), 1201);
        assert_eq!(after_second[0].quality, "SYNCED");
        assert_eq!(after_second[0].best_ask.as_deref(), Some("0.75"));
        assert_eq!(after_second[0].declared_best_mismatch, Some(false));
    }

    #[test]
    fn detecta_un_libro_cruzado_como_hueco_real() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        let book = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.40", "size": "100"}],
            "asks": [{"price": "0.60", "size": "100"}],
            "timestamp": "1000",
        });
        reconstructor.apply_clob_message(&book, Some(1000), 1001);

        // Un delta que deja el bid al mismo precio que el ask (o por encima) es imposible en un
        // libro sano: implica que faltó al menos un price_change que debió mover o vaciar el ask.
        let delta = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.60", "size": "5", "best_bid": "0.60", "best_ask": "0.60"}
            ],
        });
        let after = reconstructor.apply_clob_message(&delta, Some(1200), 1201);
        assert_eq!(after[0].quality, "CROSSED");
        assert_eq!(after[0].quality_reason, Some("crossed_book"));

        // El hueco persiste hasta el siguiente `book`, incluso si un delta posterior ya no cruza.
        let uncross = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.60", "size": "0", "best_bid": "0.40", "best_ask": "0.60"}
            ],
        });
        let still_crossed = reconstructor.apply_clob_message(&uncross, Some(1300), 1301);
        assert_eq!(still_crossed[0].quality, "CROSSED");

        let resync = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.40", "size": "100"}],
            "asks": [{"price": "0.60", "size": "100"}],
            "timestamp": "1400",
        });
        let after_resync = reconstructor.apply_clob_message(&resync, Some(1400), 1401);
        assert_eq!(after_resync[0].quality, "SYNCED");
    }

    #[test]
    fn marca_stale_tras_un_silencio_largo_y_se_limpia_con_el_siguiente_evento() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        let book = json!({
            "event_type": "book",
            "asset_id": "up",
            "bids": [{"price": "0.40", "size": "100"}],
            "asks": [{"price": "0.60", "size": "100"}],
            "timestamp": "1000",
        });
        reconstructor.apply_clob_message(&book, Some(1000), 1_000);

        let delta = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.41", "size": "5", "best_bid": "0.41", "best_ask": "0.60"}
            ],
        });
        let after = reconstructor.apply_clob_message(&delta, Some(60_000), 60_000);
        assert_eq!(after[0].quality, "STALE");
        assert_eq!(after[0].quality_reason, Some("idle_before_update"));

        let next = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.42", "size": "5", "best_bid": "0.42", "best_ask": "0.60"}
            ],
        });
        let recovered = reconstructor.apply_clob_message(&next, Some(60_500), 60_500);
        assert_eq!(recovered[0].quality, "SYNCED");
    }

    #[test]
    fn un_price_change_puede_tocar_varios_assets_a_la_vez() {
        let mut reconstructor = L2Reconstructor::new(30_000);
        for asset in ["up", "down"] {
            let book = json!({
                "event_type": "book",
                "asset_id": asset,
                "bids": [{"price": "0.40", "size": "10"}],
                "asks": [{"price": "0.60", "size": "10"}],
                "timestamp": "1000",
            });
            reconstructor.apply_clob_message(&book, Some(1000), 1001);
        }
        let complementary = json!({
            "event_type": "price_change",
            "price_changes": [
                {"asset_id": "up", "side": "BUY", "price": "0.01", "size": "5", "best_bid": "0.40", "best_ask": "0.60"},
                {"asset_id": "down", "side": "SELL", "price": "0.99", "size": "5", "best_bid": "0.40", "best_ask": "0.60"}
            ],
        });
        let updates = reconstructor.apply_clob_message(&complementary, Some(1500), 1501);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].asset_id, "up");
        assert_eq!(updates[1].asset_id, "down");
    }
}
