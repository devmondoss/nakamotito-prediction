//! Replay local y determinista de un archivo RAW de Prediction.
//!
//! No abre conexiones, no entrena modelos y no puede enviar órdenes.

use anyhow::{Context, Result};
use nakamotito_prediction::research::replay_file;
use std::path::PathBuf;

struct Args {
    input: PathBuf,
    output: PathBuf,
}

fn args() -> Result<Args> {
    let mut values = std::env::args().skip(1);
    let mut input = None;
    let mut output = None;
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--input" => input = values.next().map(PathBuf::from),
            "--out" => output = values.next().map(PathBuf::from),
            _ => anyhow::bail!("argumento no reconocido: {argument}; uso: --input events.jsonl --out normalized.jsonl"),
        }
    }
    Ok(Args {
        input: input.context("falta --input")?,
        output: output.context("falta --out")?,
    })
}

fn main() -> Result<()> {
    let args = args()?;
    let count = replay_file(&args.input, &args.output)?;
    println!(
        "replay completado: {count} eventos → {}",
        args.output.display()
    );
    Ok(())
}
