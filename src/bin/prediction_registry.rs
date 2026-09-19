//! Registry por contrato — reconstruye regla, ventana y un label de investigación por
//! ronda cruzando una sesión Gamma y una sesión RTDS ya capturadas. Sin red, sin
//! ejecución. Ver `nakamotito_prediction::registry` para el alcance exacto del label.

use anyhow::{Context, Result};
use nakamotito_prediction::registry::{build_registry, write_registry};
use std::path::PathBuf;

struct Args {
    gamma_input: PathBuf,
    rtds_input: PathBuf,
    output: PathBuf,
}

fn args() -> Result<Args> {
    let mut values = std::env::args().skip(1);
    let mut gamma_input = None;
    let mut rtds_input = None;
    let mut output = None;
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--gamma-input" => gamma_input = values.next().map(PathBuf::from),
            "--rtds-input" => rtds_input = values.next().map(PathBuf::from),
            "--out" => output = values.next().map(PathBuf::from),
            _ => anyhow::bail!(
                "argumento no reconocido: {argument}; uso: --gamma-input events.jsonl --rtds-input events.jsonl --out registry.jsonl"
            ),
        }
    }
    Ok(Args {
        gamma_input: gamma_input.context("falta --gamma-input")?,
        rtds_input: rtds_input.context("falta --rtds-input")?,
        output: output.context("falta --out")?,
    })
}

fn main() -> Result<()> {
    let args = args()?;
    let entries = build_registry(&args.gamma_input, &args.rtds_input)?;
    write_registry(&entries, &args.output)?;
    println!(
        "registry completado: {} rondas → {}",
        entries.len(),
        args.output.display()
    );
    Ok(())
}
