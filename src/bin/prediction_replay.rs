//! Replay local y determinista de un archivo RAW de Prediction.
//!
//! No abre conexiones, no entrena modelos y no puede enviar órdenes.

use anyhow::{Context, Result};
use nakamotito_prediction::research::{replay_file, replay_l2};
use std::path::PathBuf;

/// Umbral de inactividad por asset antes de marcar un update `STALE`; ver `src/l2.rs`. Es una
/// observación inicial sobre sesiones cortas, no un valor validado con campañas largas.
const DEFAULT_L2_STALE_AFTER_MS: i64 = 30_000;

struct Args {
    input: PathBuf,
    output: PathBuf,
    l2_output: Option<PathBuf>,
}

fn args() -> Result<Args> {
    let mut values = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    let mut l2_output = None;
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--input" => input = values.next().map(PathBuf::from),
            "--out" => output = values.next().map(PathBuf::from),
            "--l2-out" => l2_output = values.next().map(PathBuf::from),
            _ => anyhow::bail!(
                "argumento no reconocido: {argument}; uso: --input events.jsonl --out normalized.jsonl [--l2-out l2.jsonl]"
            ),
        }
    }
    Ok(Args {
        input: input.context("falta --input")?,
        output: output.context("falta --out")?,
        l2_output,
    })
}

fn main() -> Result<()> {
    let args = args()?;
    let count = replay_file(&args.input, &args.output)?;
    println!(
        "replay completado: {count} eventos → {}",
        args.output.display()
    );
    if let Some(l2_output) = args.l2_output {
        let l2_count = replay_l2(&args.input, &l2_output, DEFAULT_L2_STALE_AFTER_MS)?;
        println!(
            "reconstrucción L2 completada: {l2_count} updates → {}",
            l2_output.display()
        );
    }
    Ok(())
}
