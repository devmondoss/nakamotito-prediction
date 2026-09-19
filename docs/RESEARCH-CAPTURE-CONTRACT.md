# Contrato de captura de investigación v1

**Estado:** implementado para Chainlink/Polymarket; read-only; sin ejecución.

## Alcance

Los watchers `prediction_watch` y `prediction_clob_watch` aceptan `--record <directorio>`. Cada
uno conserva la respuesta RAW de su proveedor en una sesión separada:

```text
prediction-data/
  raw/
    <created-at-ms>-polymarket-rtds-chainlink/
      manifest.json
      events.jsonl
    <created-at-ms>-polymarket-gamma-clob/
      manifest.json
      events.jsonl
```

El manifest identifica versión de schema, fuente, creación, storage y que `execution=false`.
`events.jsonl` es append-only durante la sesión. El usuario controla la retención del directorio;
el repo ignora `prediction-data/` y nunca versiona el dataset.

## Evento RAW

```json
{
  "schema": "prediction-raw-event-v1",
  "source": "polymarket-rtds-chainlink",
  "kind": "rtds-message",
  "received_at_ms": 0,
  "processed_at_ms": 0,
  "raw": {}
}
```

`raw` es el payload del proveedor sin normalización. Un mensaje no JSON se conserva como texto.
`received_at_ms` se toma antes de parsear; `processed_at_ms` al serializar. Ninguno sustituye un
timestamp de evento/publicación que pueda traer el payload.

## Fuentes actuales

| Source | Kind | Contenido |
|---|---|---|
| `polymarket-rtds-chainlink` | `rtds-message` | mensajes públicos RTDS de Chainlink/TWAP |
| `polymarket-gamma-clob` | `gamma-market-response` | respuesta de descubrimiento/rule metadata de Gamma |
| `polymarket-gamma-clob` | `clob-message` | mensajes públicos de libro/trades del CLOB |

La extracción de Gamma ya publica junto al snapshot la fuente de resolución, ventana TWAP,
inicio/cierre y flags de fee declarados para la ronda. Es un registry observado por ronda, no una
asunción global ni un cálculo de coste final.

El watcher Chainlink publica además una `round_baseline` de investigación: solo captura el primer
TWAP observado durante los primeros cinco segundos de una ronda de 5 minutos. Si el watcher llega
tarde, publica `MISSED_START_TWAP` y deja la baseline nula; nunca sustituye el precio de inicio por
un valor de mitad de ronda. Esta baseline es observabilidad para el replay, no una afirmación de
que replique la lógica final de resolución.

## Límites v1

- `prediction_replay --input events.jsonl --out normalized.jsonl` implementa replay determinista
  **source-local** sin red y conserva `event_at_ms` únicamente cuando el payload lo declara.
- La retención, hash/checksum por chunk, Parquet, labels de resolución y replay cross-source son
  fases posteriores del plan.
- El writer desacopla flush a un máximo aproximado de un segundo; el evento conserva ambos relojes
  para que esa decisión sea visible al replay.
- La calidad de secuencia, gaps y cross-venue todavía no se publica como dataset normalizado.
- No hay credenciales, claves, wallet, router ni acción sobre ningún venue.
