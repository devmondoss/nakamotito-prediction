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

#[derive(Debug, Serialize)]
pub struct NormalizedEvent<'a> {
    pub schema: &'static str,
    pub source: &'a str,
    pub kind: &'a str,
    pub event_at_ms: Option<i64>,
    pub received_at_ms: i64,
    pub processed_at_ms: i64,
    pub clock_quality: &'static str,
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

fn timestamp(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
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
    let clock_quality = if event_at_ms.is_some() {
        "SOURCE_EVENT_TIME"
    } else {
        "RECEIVE_PROCESS_ONLY"
    };
    NormalizedEvent {
        schema: "prediction-normalized-event-v1",
        source: &raw.source,
        kind: &raw.kind,
        event_at_ms,
        received_at_ms: raw.received_at_ms,
        processed_at_ms: raw.processed_at_ms,
        clock_quality,
        payload: &raw.raw,
    }
}

/// Relee un JSONL RAW sin red, valida el schema de ingreso y genera JSONL normalizado. La salida
/// no sustituye ni modifica el RAW.
pub fn replay_file(input: &Path, output: &Path) -> Result<usize> {
    let reader = BufReader::new(
        File::open(input).with_context(|| format!("no se pudo abrir {}", input.display()))?,
    );
    if let Some(parent) = output.parent() {
        create_dir_all(parent).with_context(|| format!("no se pudo crear {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(output).with_context(|| format!("no se pudo crear {}", output.display()))?,
    );
    let mut count = 0_usize;
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw: RawEventOwned = serde_json::from_str(&line)
            .with_context(|| format!("RAW inválido en {}:{}", input.display(), line_number + 1))?;
        if raw.schema != "prediction-raw-event-v1" {
            anyhow::bail!(
                "schema RAW no soportado en {}:{}: {}",
                input.display(),
                line_number + 1,
                raw.schema
            );
        }
        serde_json::to_writer(&mut writer, &normalize(&raw))?;
        writer.write_all(b"\n")?;
        count += 1;
    }
    writer.flush()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(event.clock_quality, "SOURCE_EVENT_TIME");
    }
}
