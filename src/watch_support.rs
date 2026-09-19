//! Utilidades compartidas entre watchers de venues externos (Binance, Bybit). No abre
//! conexiones ni decide semántica de mercado: solo backoff acotado y escritura atómica
//! de snapshots efímeros, para no repetir esta plomería en cada collector nuevo.

use anyhow::{Context, Result};
use serde::Serialize;
use std::{path::Path, time::Duration};

/// Backoff exponencial acotado; misma fórmula que ya usaba `prediction_clob_watch`.
/// Intencionalmente limitado: la recuperación sigue siendo rápida cuando el servicio
/// vuelve.
pub fn retry_delay(reconnects: u64) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(reconnects.min(4) as u32).min(30))
}

/// Escribe `value` como JSON de forma atómica (archivo temporal + rename) para que un
/// lector nunca vea un snapshot a medio escribir.
pub fn write_snapshot_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("no se pudo crear {}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_es_acotado() {
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
        assert_eq!(retry_delay(9), Duration::from_secs(16));
    }
}
