# Nakamotito Prediction Console

Aplicación independiente de **Nakamotito Trading**. Observa oracles, rondas y libros de mercados
de predicción; no muestra el gráfico/DOM de Trading, no reutiliza su WAL ni puede enviar órdenes.

> **¿Primera vez en este repo, con cualquier agente (Claude, Codex, Antigravity, etc.)?** Empezar
> por [`docs/BITACORA-2026-09-19.md`](docs/BITACORA-2026-09-19.md): panorama completo,
> autocontenido, sin depender de historial de conversación previo.

## Estado actual

Seis watchers read-only y efímeros, todos sobre BTC:

- `prediction_watch`: BTC/USD Chainlink TWAP 60s vía Polymarket RTDS (referencia de settlement).
- `prediction_clob_watch`: ronda BTC Up/Down 5m vigente vía Gamma + Market Channel del CLOB.
- `prediction_binance_spot_watch` / `prediction_binance_perp_watch`: BTC/USDT spot y perp de Binance.
- `prediction_bybit_spot_watch` / `prediction_bybit_perp_watch`: BTC/USDT spot y perp de Bybit.

Los cuatro últimos son el multi-venue mínimo de la fase 3 del plan (formación de precio,
independiente del oracle) — ver [MULTI-VENUE-CONTRACT-V1.md](docs/MULTI-VENUE-CONTRACT-V1.md).

Todos publican snapshots reemplazables en `prediction-state/`, con vencimiento explícito. No hay
historial, base de datos, wallet, API key ni ejecución.

```bash
cargo test
cargo run --release --bin prediction_watch
cargo run --release --bin prediction_clob_watch
cargo run --release --bin prediction_binance_spot_watch
cargo run --release --bin prediction_binance_perp_watch
cargo run --release --bin prediction_bybit_spot_watch
cargo run --release --bin prediction_bybit_perp_watch
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

Los watchers de Binance/Bybit aceptan el mismo `--record`. Cada sesión crea
`prediction-data/raw/<timestamp>-<source>/manifest.json` y `events.jsonl`.
El almacén es append-only y read-only respecto a los venues: no contiene credenciales ni habilita
órdenes. Para verificar una captura sin red y escribir su versión normalizada:

```bash
cargo run --release --bin prediction_replay -- \
  --input prediction-data/raw/<session>/events.jsonl \
  --out prediction-data/normalized/<session>.jsonl \
  --l2-out prediction-data/normalized/<session>-l2.jsonl
```

`--l2-out` es opcional y reconstruye determinísticamente el libro L2 de una sesión CLOB (`book` +
`price_change`), sin red. El contrato de normalización está en
[normalized-event-v1](docs/NORMALIZED-EVENT-CONTRACT-V1.md) y el de la reconstrucción de libro en
[CLOB L2](docs/CLOB-L2-CONTRACT-V1.md).

Para reconstruir la regla de resolución, ventana y un label de investigación por ronda (BTC
Up/Down), cruzando una sesión Gamma con su sesión RTDS correspondiente:

```bash
cargo run --release --bin prediction_registry -- \
  --gamma-input prediction-data/raw/<sesión>-polymarket-gamma-clob/events.jsonl \
  --rtds-input prediction-data/raw/<sesión>-polymarket-rtds-chainlink/events.jsonl \
  --out prediction-data/registry/<sesión>.jsonl
```

El label es una reconstrucción propia sobre lo capturado, no la resolución oficial de
Polymarket; sin evidencia suficiente queda `UNRESOLVED`, nunca inferido. Contrato completo en
[CONTRACT-REGISTRY-V1.md](docs/CONTRACT-REGISTRY-V1.md). Los próximos pasos siguen en el
[plan cuantitativo](docs/QUANT-SETTLEMENT-PLAN-2026-09-18.md).

Antes de añadir cualquier forecast se validará la salud del feed, transición de rondas, timestamps
y libro vigente.

## Knowledge Graph

Este repo comparte el grafo privado [Nakamotito Knowledge](docs/KNOWLEDGE_GRAPH.md) con
Nakamotito Trading; comparte conocimiento, no runtime ni interfaz.

Para retomar trabajo, leer primero el [relevo Quant más reciente](docs/HANDOFF-QUANT-2026-09-19.md). Los relevos anteriores ([2026-09-18](docs/HANDOFF-QUANT-2026-09-18.md), [2026-09-09](docs/HANDOFF-2026-09-09.md)) quedan como contexto histórico.
