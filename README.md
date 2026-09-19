# Nakamotito Prediction Console

Aplicación independiente de **Nakamotito Trading**. Observa oracles, rondas y libros de mercados
de predicción; no muestra el gráfico/DOM de Trading, no reutiliza su WAL ni puede enviar órdenes.

## Estado actual

Dos watchers read-only y efímeros:

- `prediction_watch`: BTC/USD Chainlink TWAP 60s vía Polymarket RTDS.
- `prediction_clob_watch`: ronda BTC Up/Down 5m vigente vía Gamma + Market Channel del CLOB.

Ambos publican snapshots reemplazables en `prediction-state/`, con vencimiento explícito. No hay
historial, base de datos, wallet, API key ni ejecución.

```bash
cargo test
cargo run --release --bin prediction_watch
cargo run --release --bin prediction_clob_watch
```

## Consola local

La consola independiente inicia y detiene sus propios watchers, captura sus payloads RAW de
investigación en `prediction-data/` y expone el resultado en `http://127.0.0.1:3090`:

```bash
cargo run --release --bin prediction_console
```

Muestra la referencia de settlement (Chainlink TWAP), ronda BTC Up/Down vigente, tiempo al cierre,
mejor bid/ask/spread de cada outcome y relojes, frescura, reconexiones y errores de ambos feeds.
El botón `START WATCHERS` inicia los conectores y la captura RAW; `STOP` mata solamente los hijos
de la consola y elimina los snapshots efímeros, pero conserva el dataset de investigación. La
consola no consume el runtime de Trading y no puede enviar órdenes.

## Captura RAW para investigación

La persistencia de investigación fue aprobada el 2026-09-18. Cada watcher puede ejecutarse de
forma independiente y almacenar el payload del proveedor junto con `received_at_ms` y
`processed_at_ms`:

```bash
cargo run --release --bin prediction_watch -- --record prediction-data
cargo run --release --bin prediction_clob_watch -- --record prediction-data
```

Cada sesión crea `prediction-data/raw/<timestamp>-<source>/manifest.json` y `events.jsonl`.
El almacén es append-only y read-only respecto a los venues: no contiene credenciales ni habilita
órdenes. Para verificar una captura sin red y escribir su versión normalizada:

```bash
cargo run --release --bin prediction_replay -- \
  --input prediction-data/raw/<session>/events.jsonl \
  --out prediction-data/normalized/<session>.jsonl
```

El contrato y próximos pasos están en el [plan cuantitativo](docs/QUANT-SETTLEMENT-PLAN-2026-09-18.md).

Antes de añadir cualquier forecast se validará la salud del feed, transición de rondas, timestamps
y libro vigente.

## Knowledge Graph

Este repo comparte el grafo privado [Nakamotito Knowledge](docs/KNOWLEDGE_GRAPH.md) con
Nakamotito Trading; comparte conocimiento, no runtime ni interfaz.

Para retomar trabajo, leer primero el [relevo Quant actualizado](docs/HANDOFF-QUANT-2026-09-18.md). El [relevo anterior](docs/HANDOFF-2026-09-09.md) queda como contexto histórico.
