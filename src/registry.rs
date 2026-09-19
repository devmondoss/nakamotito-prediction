//! Registry por contrato: cruza una sesión RAW `polymarket-gamma-clob` (regla y ventana
//! por ronda) con una sesión RAW `polymarket-rtds-chainlink` (TWAP observado) para
//! reconstruir, por ronda, la regla de resolución y un label auto-calculado.
//!
//! El label **no es la resolución autoritativa de Polymarket ni on-chain**: es una
//! reconstrucción de investigación a partir de lo que capturamos nosotros mismos, con la
//! misma regla que Polymarket documenta ("Up si el TWAP de cierre >= el TWAP de
//! apertura"). Sin evidencia suficiente en el RAW capturado, el label queda
//! `UNRESOLVED` con la razón exacta — nunca se infiere un valor faltante.

use crate::research::{for_each_raw_event, timestamp, RawEventOwned};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::{create_dir_all, File},
    io::{BufWriter, Write},
    path::Path,
};

/// Antes de esto tras el inicio de ronda, un TWAP cuenta como baseline de apertura;
/// igual que `prediction_watch::START_CAPTURE_GRACE_MS`.
const BASELINE_GRACE_MS: i64 = 5_000;
/// Una lectura debe estar dentro de esta ventana antes del fin de ronda para contar
/// como "cierre". Más vieja que esto es evidencia insuficiente: no se extrapola un
/// valor de mitad de ronda como si fuera el cierre.
const CLOSING_GRACE_MS: i64 = 10_000;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct RegistryEntry {
    pub schema: &'static str,
    pub slug: String,
    pub question: Option<String>,
    pub resolution_source: Option<String>,
    pub round_start_ms: i64,
    pub round_end_ms: i64,
    pub twap_window_seconds: Option<u64>,
    pub fee_type: Option<String>,
    pub fees_enabled: Option<bool>,
    pub baseline_twap: Option<String>,
    pub baseline_observed_at_ms: Option<i64>,
    pub closing_twap: Option<String>,
    pub closing_observed_at_ms: Option<i64>,
    /// `UP` | `DOWN` | `UNRESOLVED`. Reconstrucción de investigación, no la resolución
    /// autoritativa de Polymarket/on-chain — ver el comentario del módulo.
    pub label: &'static str,
    pub label_reason: Option<&'static str>,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

/// El inicio/fin de ronda vienen del propio slug (`btc-updown-5m-<segundos>`), igual que
/// el descubrimiento en vivo (`prediction_clob_watch::current_slug`). No se parsean las
/// fechas ISO-8601 de Gamma para esto: son redundantes con el slug y evitarlo ahorra
/// sumar una dependencia de fechas solo para esta cuenta.
fn round_window_from_slug(slug: &str) -> Option<(i64, i64)> {
    let seconds: i64 = slug.strip_prefix("btc-updown-5m-")?.parse().ok()?;
    let start = seconds * 1000;
    Some((start, start + 300_000))
}

struct PartialRule {
    question: Option<String>,
    resolution_source: Option<String>,
    twap_window_seconds: Option<u64>,
    fee_type: Option<String>,
    fees_enabled: Option<bool>,
}

fn parse_gamma_rule(raw: &Value) -> Option<(String, PartialRule)> {
    let market = raw.as_array()?.first()?;
    let slug = market.get("slug")?.as_str()?.to_owned();
    Some((
        slug,
        PartialRule {
            question: market
                .get("question")
                .and_then(Value::as_str)
                .map(str::to_owned),
            resolution_source: market
                .get("resolutionSource")
                .and_then(Value::as_str)
                .map(str::to_owned),
            twap_window_seconds: market
                .get("cryptoMarketConfig")
                .and_then(|value| value.get("twapLookbackSeconds"))
                .and_then(Value::as_u64),
            fee_type: market
                .get("feeType")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fees_enabled: market.get("feesEnabled").and_then(Value::as_bool),
        },
    ))
}

/// Lee una sesión RAW `polymarket-gamma-clob` y devuelve una regla por slug distinto
/// visto en sus eventos `gamma-market-response`. Una sesión larga puede ver varias
/// rondas seguidas; se conserva la primera respuesta vista por slug.
fn collect_rules(gamma_input: &Path) -> Result<Vec<(String, PartialRule)>> {
    let mut rules: Vec<(String, PartialRule)> = Vec::new();
    let mut seen = HashSet::new();
    for_each_raw_event(gamma_input, |raw: RawEventOwned| {
        if raw.kind != "gamma-market-response" {
            return Ok(());
        }
        if let Some((slug, rule)) = parse_gamma_rule(&raw.raw) {
            if seen.insert(slug.clone()) {
                rules.push((slug, rule));
            }
        }
        Ok(())
    })?;
    Ok(rules)
}

/// Lee una sesión RAW `polymarket-rtds-chainlink` y devuelve `(observed_at_ms, twap)`
/// ordenado por tiempo de observación Chainlink (no de recepción/publicación).
fn collect_twap_series(rtds_input: &Path) -> Result<Vec<(i64, String)>> {
    let mut series = Vec::new();
    for_each_raw_event(rtds_input, |raw: RawEventOwned| {
        if raw.kind != "rtds-message" {
            return Ok(());
        }
        let Some(payload) = raw.raw.get("payload") else {
            return Ok(());
        };
        if payload.get("symbol").and_then(Value::as_str) != Some("btc/usd") {
            return Ok(());
        }
        let Some(observed_at_ms) = timestamp(payload.get("timestamp")) else {
            return Ok(());
        };
        let Some(value) = text(payload.get("value")) else {
            return Ok(());
        };
        series.push((observed_at_ms, value));
        Ok(())
    })?;
    series.sort_by_key(|(at, _)| *at);
    Ok(series)
}

fn baseline_in(series: &[(i64, String)], round_start_ms: i64) -> Option<(i64, String)> {
    series
        .iter()
        .find(|(at, _)| *at >= round_start_ms && *at <= round_start_ms + BASELINE_GRACE_MS)
        .cloned()
}

fn closing_in(series: &[(i64, String)], round_end_ms: i64) -> Option<(i64, String)> {
    series
        .iter()
        .rev()
        .find(|(at, _)| *at <= round_end_ms && *at >= round_end_ms - CLOSING_GRACE_MS)
        .cloned()
}

/// Construye el registry cruzando una sesión Gamma y una sesión RTDS ya capturadas. Sin
/// red. Ver el comentario del módulo sobre el alcance del `label`.
pub fn build_registry(gamma_input: &Path, rtds_input: &Path) -> Result<Vec<RegistryEntry>> {
    let rules = collect_rules(gamma_input)?;
    let series = collect_twap_series(rtds_input)?;
    let mut entries = Vec::with_capacity(rules.len());
    for (slug, rule) in rules {
        let Some((round_start_ms, round_end_ms)) = round_window_from_slug(&slug) else {
            continue;
        };
        let baseline = baseline_in(&series, round_start_ms);
        let closing = closing_in(&series, round_end_ms);
        let (label, label_reason) = match (&baseline, &closing) {
            (Some((_, base)), Some((_, close))) => {
                match (base.parse::<f64>(), close.parse::<f64>()) {
                    (Ok(base), Ok(close)) if close >= base => ("UP", None),
                    (Ok(_), Ok(_)) => ("DOWN", None),
                    _ => ("UNRESOLVED", Some("unparseable_twap")),
                }
            }
            (None, _) => ("UNRESOLVED", Some("missing_baseline")),
            (Some(_), None) => ("UNRESOLVED", Some("missing_closing")),
        };
        entries.push(RegistryEntry {
            schema: "prediction-round-registry-v1",
            slug,
            question: rule.question,
            resolution_source: rule.resolution_source,
            round_start_ms,
            round_end_ms,
            twap_window_seconds: rule.twap_window_seconds,
            fee_type: rule.fee_type,
            fees_enabled: rule.fees_enabled,
            baseline_twap: baseline.as_ref().map(|(_, v)| v.clone()),
            baseline_observed_at_ms: baseline.as_ref().map(|(at, _)| *at),
            closing_twap: closing.as_ref().map(|(_, v)| v.clone()),
            closing_observed_at_ms: closing.as_ref().map(|(at, _)| *at),
            label,
            label_reason,
        });
    }
    Ok(entries)
}

/// Escribe el registry como JSONL, una línea por ronda.
pub fn write_registry(entries: &[RegistryEntry], output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        create_dir_all(parent).with_context(|| format!("no se pudo crear {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(output).with_context(|| format!("no se pudo crear {}", output.display()))?,
    );
    for entry in entries {
        serde_json::to_writer(&mut writer, entry)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrae_ventana_de_ronda_desde_el_slug() {
        assert_eq!(
            round_window_from_slug("btc-updown-5m-1789760100"),
            Some((1789760100_000, 1789760400_000))
        );
        assert_eq!(round_window_from_slug("otra-cosa"), None);
    }

    #[test]
    fn etiqueta_up_cuando_el_cierre_no_baja_del_baseline() {
        let series = vec![
            (1_000_i64, "100.0".to_string()),
            (1_002_000, "101.5".to_string()),
        ];
        assert_eq!(baseline_in(&series, 0), Some((1_000, "100.0".to_string())));
        assert_eq!(
            closing_in(&series, 1_005_000),
            Some((1_002_000, "101.5".to_string()))
        );
    }

    #[test]
    fn sin_lectura_de_cierre_dentro_de_gracia_queda_unresolved() {
        // La última lectura es muy anterior al fin de ronda: no se extrapola.
        let series = vec![(1_000_i64, "100.0".to_string())];
        assert_eq!(closing_in(&series, 1_000_000), None);
    }

    fn write_lines(path: &Path, lines: &[String]) {
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn construye_el_registry_cruzando_gamma_y_rtds() {
        let dir = std::env::temp_dir().join(format!(
            "prediction-registry-test-{}",
            crate::research::now_ms()
        ));
        create_dir_all(&dir).unwrap();

        let gamma_path = dir.join("gamma.jsonl");
        let gamma_response = serde_json::json!([{
            "slug": "btc-updown-5m-1000",
            "question": "Bitcoin Up or Down",
            "resolutionSource": "chainlink",
            "cryptoMarketConfig": {"twapEnabled": true, "twapLookbackSeconds": 60},
            "feeType": "crypto",
            "feesEnabled": true,
        }]);
        write_lines(
            &gamma_path,
            &[serde_json::json!({
                "schema": "prediction-raw-event-v1",
                "source": "polymarket-gamma-clob",
                "kind": "gamma-market-response",
                "received_at_ms": 1,
                "processed_at_ms": 1,
                "raw": gamma_response,
            })
            .to_string()],
        );

        let rtds_path = dir.join("rtds.jsonl");
        let mut rtds_lines = Vec::new();
        for (observed_at_ms, value) in [
            (1_000_000_i64, "65000.0"),
            (1_299_500, "65100.0"), // dentro de la ventana de cierre (round_end=1_300_000)
        ] {
            rtds_lines.push(
                serde_json::json!({
                    "schema": "prediction-raw-event-v1",
                    "source": "polymarket-rtds-chainlink",
                    "kind": "rtds-message",
                    "received_at_ms": observed_at_ms,
                    "processed_at_ms": observed_at_ms,
                    "raw": {
                        "topic": "crypto_prices_twap_sixty",
                        "timestamp": observed_at_ms,
                        "payload": {"symbol": "btc/usd", "value": value, "timestamp": observed_at_ms, "window_s": 60},
                    },
                })
                .to_string(),
            );
        }
        write_lines(&rtds_path, &rtds_lines);

        let entries = build_registry(&gamma_path, &rtds_path).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.slug, "btc-updown-5m-1000");
        assert_eq!(entry.round_start_ms, 1_000_000);
        assert_eq!(entry.round_end_ms, 1_300_000);
        assert_eq!(entry.baseline_twap.as_deref(), Some("65000.0"));
        assert_eq!(entry.closing_twap.as_deref(), Some("65100.0"));
        assert_eq!(entry.label, "UP");
        assert_eq!(entry.label_reason, None);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ronda_sin_baseline_queda_unresolved_no_inferida() {
        let dir = std::env::temp_dir().join(format!(
            "prediction-registry-test-nobaseline-{}",
            crate::research::now_ms()
        ));
        create_dir_all(&dir).unwrap();

        let gamma_path = dir.join("gamma.jsonl");
        let gamma_response = serde_json::json!([{
            "slug": "btc-updown-5m-2000",
            "question": "Bitcoin Up or Down",
        }]);
        write_lines(
            &gamma_path,
            &[serde_json::json!({
                "schema": "prediction-raw-event-v1",
                "source": "polymarket-gamma-clob",
                "kind": "gamma-market-response",
                "received_at_ms": 1,
                "processed_at_ms": 1,
                "raw": gamma_response,
            })
            .to_string()],
        );
        // El watcher arrancó tarde: la primera lectura llega mucho después del inicio.
        let rtds_path = dir.join("rtds.jsonl");
        write_lines(
            &rtds_path,
            &[serde_json::json!({
                "schema": "prediction-raw-event-v1",
                "source": "polymarket-rtds-chainlink",
                "kind": "rtds-message",
                "received_at_ms": 2_100_000_i64,
                "processed_at_ms": 2_100_000_i64,
                "raw": {
                    "topic": "crypto_prices_twap_sixty",
                    "timestamp": 2_100_000_i64,
                    "payload": {"symbol": "btc/usd", "value": "1", "timestamp": 2_100_000_i64, "window_s": 60},
                },
            })
            .to_string()],
        );

        let entries = build_registry(&gamma_path, &rtds_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "UNRESOLVED");
        assert_eq!(entries[0].label_reason, Some("missing_baseline"));

        std::fs::remove_dir_all(dir).unwrap();
    }
}
