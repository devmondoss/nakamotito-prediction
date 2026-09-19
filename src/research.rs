//! Event store RAW append-only para investigación read-only.
//!
//! Mantiene los clocks de recepción y proceso fuera del payload del proveedor. No normaliza ni
//! infiere semántica: esa capa pertenece al replay/normalizador posterior.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{create_dir_all, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

const FLUSH_INTERVAL_MS: i64 = 1_000;

#[derive(Debug, Serialize)]
struct SessionManifest<'a> {
    schema: &'static str,
    session_id: &'a str,
    source: &'a str,
    created_at_ms: i64,
    execution: bool,
    storage: &'static str,
    retention: &'static str,
}

#[derive(Debug, Serialize)]
struct RawEvent<'a> {
    schema: &'static str,
    source: &'a str,
    kind: &'a str,
    received_at_ms: i64,
    processed_at_ms: i64,
    raw: &'a Value,
}

/// Forma estable para replay. `event_at_ms` viene únicamente del payload de origen y queda nulo
/// cuando la fuente no declaró un reloj de evento interpretable.
#[derive(Debug, Deserialize)]
pub struct RawEventOwned {
    pub schema: String,
    pub source: String,
    pub kind: String,
    pub received_at_ms: i64,
    pub processed_at_ms: i64,
    pub raw: Value,
}

/// `normalized-event-v1`: contrato de calidad interfuente. Nunca infiere un reloj o secuencia que
/// el proveedor no declaró; cuando falta, el campo queda `None` y la razón documenta por qué.
#[derive(Debug, Serialize)]
pub struct NormalizedEvent<'a> {
    pub schema: &'static str,
    pub source: &'a str,
    pub kind: &'a str,
    /// Reloj de evento tal como lo declaró el proveedor (p. ej. el `timestamp` de Chainlink dentro
    /// del payload, o el `timestamp` de un mensaje del CLOB). `None` si no lo declaró.
    pub event_at_ms: Option<i64>,
    pub received_at_ms: i64,
    pub processed_at_ms: i64,
    /// `processed_at_ms - received_at_ms`: latencia local determinista, siempre calculable.
    pub ingest_latency_ms: i64,
    /// `received_at_ms - event_at_ms`: solo cuando el proveedor declaró un reloj de evento.
    pub publish_latency_ms: Option<i64>,
    /// Secuencia/epoch opaco del proveedor si lo expone. Ninguna fuente actual lo declara, así
    /// que hoy siempre es `None`; queda reservado para cuando exista una fuente que sí lo dé.
    pub sequence: Option<i64>,
    pub quality: &'static str,
    /// Obligatorio cuando `quality != "GOOD"`; documenta por qué, nunca se deja implícito.
    pub quality_reason: Option<&'static str>,
    pub payload: &'a Value,
}

/// Un writer por fuente: el nombre de sesión incluye reloj y fuente para que las capturas
/// concurrentes no se mezclen ni sobrescriban.
pub struct ResearchWriter {
    events: BufWriter<File>,
    last_flush_ms: i64,
    session_dir: PathBuf,
    source: String,
}

impl ResearchWriter {
    pub fn open(root: &Path, source: &str, created_at_ms: i64) -> Result<Self> {
        let session_id = format!("{created_at_ms}-{source}");
        let session_dir = root.join("raw").join(&session_id);
        create_dir_all(&session_dir)
            .with_context(|| format!("no se pudo crear {}", session_dir.display()))?;
        let manifest = SessionManifest {
            schema: "prediction-research-session-v1",
            session_id: &session_id,
            source,
            created_at_ms,
            execution: false,
            storage: "jsonl/raw-append-only",
            retention: "until user-managed deletion",
        };
        std::fs::write(
            session_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        let events = BufWriter::new(File::create(session_dir.join("events.jsonl"))?);
        Ok(Self {
            events,
            last_flush_ms: created_at_ms,
            session_dir,
            source: source.to_owned(),
        })
    }

    pub fn record_value(&mut self, kind: &str, received_at_ms: i64, raw: &Value) -> Result<()> {
        let processed_at_ms = now_ms();
        let event = RawEvent {
            schema: "prediction-raw-event-v1",
            source: &self.source,
            kind,
            received_at_ms,
            processed_at_ms,
            raw,
        };
        serde_json::to_writer(&mut self.events, &event)?;
        self.events.write_all(b"\n")?;
        if processed_at_ms - self.last_flush_ms >= FLUSH_INTERVAL_MS {
            self.events.flush()?;
            self.last_flush_ms = processed_at_ms;
        }
        Ok(())
    }

    pub fn record_text(&mut self, kind: &str, received_at_ms: i64, raw: &str) -> Result<()> {
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()));
        self.record_value(kind, received_at_ms, &value)
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn flush(&mut self) -> Result<()> {
        self.events.flush()?;
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn timestamp(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

/// Un `kind` reconocido es uno para el que sabemos extraer un reloj de evento del payload
/// declarado; los demás (p. ej. `gamma-market-response`, `new_market`, `tick_size_change`) se
/// conservan igual pero sin inventar un reloj que el proveedor no puso en ese lugar.
fn known_event_clock_kind(kind: &str) -> bool {
    matches!(kind, "rtds-message" | "clob-message")
}

pub fn normalize(raw: &RawEventOwned) -> NormalizedEvent<'_> {
    let event_at_ms = match raw.kind.as_str() {
        "rtds-message" => raw
            .raw
            .get("payload")
            .and_then(|payload| timestamp(payload.get("timestamp"))),
        "clob-message" => timestamp(raw.raw.get("timestamp")),
        _ => None,
    };
    let ingest_latency_ms = raw.processed_at_ms - raw.received_at_ms;
    let publish_latency_ms = event_at_ms.map(|event_at_ms| raw.received_at_ms - event_at_ms);

    let (quality, quality_reason) = match (event_at_ms, publish_latency_ms) {
        _ if !known_event_clock_kind(&raw.kind) => ("DEGRADED", Some("unrecognized_kind")),
        (Some(_), Some(latency)) if latency < 0 => ("INVALID", Some("event_after_receive")),
        (Some(_), _) => ("GOOD", None),
        (None, _) => ("DEGRADED", Some("no_event_clock_declared")),
    };

    NormalizedEvent {
        schema: "prediction-normalized-event-v1",
        source: &raw.source,
        kind: &raw.kind,
        event_at_ms,
        received_at_ms: raw.received_at_ms,
        processed_at_ms: raw.processed_at_ms,
        ingest_latency_ms,
        publish_latency_ms,
        sequence: None,
        quality,
        quality_reason,
        payload: &raw.raw,
    }
}

/// Recorre un JSONL RAW línea por línea y entrega cada evento válido a `handle`. Una sesión
/// interrumpida a mitad de escritura (p. ej. el proceso murió sin cerrar limpio) deja su última
/// línea truncada; en vez de abortar toda la relectura por eso, se avisa por stderr y se ignora
/// solo esa línea final. Cualquier otra línea inválida —o un schema no soportado— sigue siendo un
/// error duro: no se puede distinguir de corrupción real.
pub(crate) fn for_each_raw_event(
    input: &Path,
    mut handle: impl FnMut(RawEventOwned) -> Result<()>,
) -> Result<()> {
    let reader = BufReader::new(
        File::open(input).with_context(|| format!("no se pudo abrir {}", input.display()))?,
    );
    let mut lines = reader.lines().enumerate();
    let mut pending = lines.next();
    while let Some((line_number, line)) = pending {
        let line = line?;
        pending = lines.next();
        let is_last_line = pending.is_none();
        if line.trim().is_empty() {
            continue;
        }
        let raw: RawEventOwned = match serde_json::from_str(&line) {
            Ok(raw) => raw,
            Err(error) if is_last_line => {
                eprintln!(
                    "aviso: {}:{} truncado (sesión probablemente interrumpida a mitad de escritura); se ignora esa última línea: {error}",
                    input.display(),
                    line_number + 1
                );
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("RAW inválido en {}:{}", input.display(), line_number + 1)
                });
            }
        };
        if raw.schema != "prediction-raw-event-v1" {
            anyhow::bail!(
                "schema RAW no soportado en {}:{}: {}",
                input.display(),
                line_number + 1,
                raw.schema
            );
        }
        handle(raw)?;
    }
    Ok(())
}

/// Relee un JSONL RAW sin red y genera JSONL normalizado. La salida no sustituye ni modifica el
/// RAW.
pub fn replay_file(input: &Path, output: &Path) -> Result<usize> {
    if let Some(parent) = output.parent() {
        create_dir_all(parent).with_context(|| format!("no se pudo crear {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(output).with_context(|| format!("no se pudo crear {}", output.display()))?,
    );
    let mut count = 0_usize;
    for_each_raw_event(input, |raw| {
        serde_json::to_writer(&mut writer, &normalize(&raw))?;
        writer.write_all(b"\n")?;
        count += 1;
        Ok(())
    })?;
    writer.flush()?;
    Ok(count)
}

/// Reconstruye determinísticamente el libro L2 de una sesión `polymarket-gamma-clob` a partir de
/// su RAW (`book` + `price_change`), sin red. Otros `kind` del mismo archivo (p. ej.
/// `gamma-market-response`) se ignoran; no aportan libro. `stale_after_ms` es el umbral de
/// inactividad por asset a partir del cual un update se marca `STALE` (ver `crate::l2`).
pub fn replay_l2(input: &Path, output: &Path, stale_after_ms: i64) -> Result<usize> {
    if let Some(parent) = output.parent() {
        create_dir_all(parent).with_context(|| format!("no se pudo crear {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(output).with_context(|| format!("no se pudo crear {}", output.display()))?,
    );
    let mut reconstructor = crate::l2::L2Reconstructor::new(stale_after_ms);
    let mut count = 0_usize;
    for_each_raw_event(input, |raw| {
        if raw.kind != "clob-message" {
            return Ok(());
        }
        let event_at_ms = timestamp(raw.raw.get("timestamp"));
        for update in reconstructor.apply_clob_message(&raw.raw, event_at_ms, raw.received_at_ms) {
            serde_json::to_writer(&mut writer, &update)?;
            writer.write_all(b"\n")?;
            count += 1;
        }
        Ok(())
    })?;
    writer.flush()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolera_una_ultima_linea_truncada_por_una_sesion_interrumpida() {
        let dir =
            std::env::temp_dir().join(format!("prediction-replay-test-truncated-{}", now_ms()));
        create_dir_all(&dir).unwrap();
        let input = dir.join("events.jsonl");
        std::fs::write(
            &input,
            "{\"schema\":\"prediction-raw-event-v1\",\"source\":\"s\",\"kind\":\"clob-message\",\"received_at_ms\":1,\"processed_at_ms\":1,\"raw\":{\"timestamp\":\"1\"}}\n{\"schema\":\"prediction-raw-event-v1\",\"source\":\"s\",\"kind\":\"clob-message\",\"received_at_ms\":2,\"processed_at_ms\":2,\"raw\":{\"time",
        )
        .unwrap();
        let output = dir.join("normalized.jsonl");
        let count = replay_file(&input, &output).unwrap();
        assert_eq!(
            count, 1,
            "la línea completa se procesa; la truncada se ignora sin abortar"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn una_linea_invalida_que_no_es_la_ultima_sigue_siendo_error_duro() {
        let dir =
            std::env::temp_dir().join(format!("prediction-replay-test-hard-error-{}", now_ms()));
        create_dir_all(&dir).unwrap();
        let input = dir.join("events.jsonl");
        std::fs::write(
            &input,
            "{ esto no es json\n{\"schema\":\"prediction-raw-event-v1\",\"source\":\"s\",\"kind\":\"clob-message\",\"received_at_ms\":2,\"processed_at_ms\":2,\"raw\":{}}\n",
        )
        .unwrap();
        let output = dir.join("normalized.jsonl");
        assert!(replay_file(&input, &output).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn conserva_payload_y_relojes_en_jsonl() {
        let root = std::env::temp_dir().join(format!("prediction-research-test-{}", now_ms()));
        let mut writer = ResearchWriter::open(&root, "test-source", 10).unwrap();
        writer
            .record_text("message", 11, r#"{"price":"1"}"#)
            .unwrap();
        writer.flush().unwrap();
        let raw = std::fs::read_to_string(writer.session_dir().join("events.jsonl")).unwrap();
        assert!(raw.contains("prediction-raw-event-v1"));
        assert!(raw.contains("test-source"));
        assert!(raw.contains("received_at_ms\":11"));
        assert!(raw.contains("price"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normaliza_reloj_de_evento_de_rtds_sin_inventarlo() {
        let raw = RawEventOwned {
            schema: "prediction-raw-event-v1".into(),
            source: "rtds".into(),
            kind: "rtds-message".into(),
            received_at_ms: 11,
            processed_at_ms: 12,
            raw: serde_json::json!({"payload":{"timestamp":10}}),
        };
        let event = normalize(&raw);
        assert_eq!(event.event_at_ms, Some(10));
        assert_eq!(event.publish_latency_ms, Some(1));
        assert_eq!(event.ingest_latency_ms, 1);
        assert_eq!(event.quality, "GOOD");
        assert_eq!(event.quality_reason, None);
        assert_eq!(event.sequence, None);
    }

    #[test]
    fn degrada_cuando_la_fuente_no_declara_reloj_de_evento() {
        let raw = RawEventOwned {
            schema: "prediction-raw-event-v1".into(),
            source: "polymarket-gamma-clob".into(),
            kind: "clob-message".into(),
            received_at_ms: 20,
            processed_at_ms: 21,
            raw: serde_json::json!({"event_type": "new_market"}),
        };
        let event = normalize(&raw);
        assert_eq!(event.event_at_ms, None);
        assert_eq!(event.publish_latency_ms, None);
        assert_eq!(event.quality, "DEGRADED");
        assert_eq!(event.quality_reason, Some("no_event_clock_declared"));
    }

    #[test]
    fn degrada_un_kind_no_reconocido_sin_descartar_el_payload() {
        let raw = RawEventOwned {
            schema: "prediction-raw-event-v1".into(),
            source: "polymarket-gamma-clob".into(),
            kind: "gamma-market-response".into(),
            received_at_ms: 20,
            processed_at_ms: 21,
            raw: serde_json::json!({"slug": "btc-updown-5m-1"}),
        };
        let event = normalize(&raw);
        assert_eq!(event.quality, "DEGRADED");
        assert_eq!(event.quality_reason, Some("unrecognized_kind"));
        assert_eq!(event.payload["slug"], "btc-updown-5m-1");
    }

    #[test]
    fn invalida_un_reloj_de_evento_posterior_a_la_recepcion() {
        let raw = RawEventOwned {
            schema: "prediction-raw-event-v1".into(),
            source: "polymarket-gamma-clob".into(),
            kind: "clob-message".into(),
            received_at_ms: 100,
            processed_at_ms: 101,
            raw: serde_json::json!({"timestamp": "150"}),
        };
        let event = normalize(&raw);
        assert_eq!(event.event_at_ms, Some(150));
        assert_eq!(event.publish_latency_ms, Some(-50));
        assert_eq!(event.quality, "INVALID");
        assert_eq!(event.quality_reason, Some("event_after_receive"));
    }
}
