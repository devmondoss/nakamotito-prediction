# Nakamotito Prediction Console

Aplicación independiente de **Nakamotito Trading**. Observa oracles, rondas y libros de mercados
de predicción; no muestra el gráfico/DOM de Trading, no reutiliza su WAL ni puede enviar órdenes.

## Estado inicial

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

La siguiente entrega es la ventana nativa propia que inicia, vigila y compone ambos watchers.
Antes de añadir cualquier forecast se validará la salud del feed, transición de rondas, timestamps
y libro vigente.
