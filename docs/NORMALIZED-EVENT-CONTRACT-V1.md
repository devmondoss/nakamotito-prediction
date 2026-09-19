# Contrato `normalized-event-v1`

**Estado:** implementado en `src/research.rs::normalize` y `prediction_replay`; read-only,
source-local, sin red. Cubre la fase 0 del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md)
("contrato de calidad") para las dos fuentes actuales; queda abierto a nuevas fuentes sin cambiar
su forma.

## Objetivo

Dar a cualquier evento —de cualquier fuente presente o futura— una forma común de identidad,
relojes, latencia, secuencia y calidad, siempre trazable al RAW del que proviene. Ninguna fuente
recibe un trato especial que oculte lo que no declaró: si un proveedor no da un reloj de evento o
una secuencia, el campo correspondiente queda `null` y la razón de la degradación queda explícita.

## Forma del evento

```json
{
  "schema": "prediction-normalized-event-v1",
  "source": "polymarket-gamma-clob",
  "kind": "clob-message",
  "event_at_ms": 1789760074559,
  "received_at_ms": 1789760074561,
  "processed_at_ms": 1789760074561,
  "ingest_latency_ms": 0,
  "publish_latency_ms": 2,
  "sequence": null,
  "quality": "GOOD",
  "quality_reason": null,
  "payload": {}
}
```

| Campo | Significado | Cuándo es `null`/`None` |
|---|---|---|
| `event_at_ms` | Reloj de evento tal como lo declaró el proveedor (nunca inferido). | El `kind` no tiene un reloj de evento conocido, o el proveedor no lo incluyó en ese mensaje. |
| `received_at_ms` / `processed_at_ms` | Relojes locales ya capturados en RAW (`src/research.rs::RawEvent`). | Nunca; siempre existen. |
| `ingest_latency_ms` | `processed_at_ms - received_at_ms`. Latencia local determinista. | Nunca. |
| `publish_latency_ms` | `received_at_ms - event_at_ms`. | Solo si `event_at_ms` es `None`. |
| `sequence` | Secuencia/epoch opaco del proveedor. | Siempre `None` hoy: ni RTDS ni el Market Channel del CLOB declaran una secuencia o número de mensaje. No se sustituye por un contador local, porque eso no demuestra ausencia de huecos. |
| `quality` | `GOOD` \| `DEGRADED` \| `INVALID` (ver tabla). `ABSENT` no aplica a nivel de evento individual —describe la ausencia de una fuente completa, que se evalúa en la capa de features, no aquí. | — |
| `quality_reason` | Código estable de la razón. | Solo cuando `quality == "GOOD"`. |

## Reglas de calidad (deterministas, sin umbrales inventados)

| `quality` | Condición | `quality_reason` |
|---|---|---|
| `GOOD` | `event_at_ms` presente y `publish_latency_ms >= 0`. | — |
| `DEGRADED` | El `kind` no tiene un reloj de evento conocido (p. ej. `gamma-market-response`, `new_market`, `tick_size_change` cuando el proveedor no puso `timestamp` en ese lugar). | `unrecognized_kind` |
| `DEGRADED` | El `kind` sí tiene reloj de evento conocido (`rtds-message`, `clob-message`) pero este mensaje concreto no lo trajo. | `no_event_clock_declared` |
| `INVALID` | `event_at_ms` posterior a `received_at_ms` (`publish_latency_ms < 0`): reloj imposible, típicamente desfase del proveedor o bug de parseo. | `event_after_receive` |

`DEGRADED` no descarta el payload: se conserva completo en `payload` para que una fase posterior
decida si lo usa. Un evento `DEGRADED` o `INVALID` no debe alimentar features ni labels sin decisión
explícita de la fase 5 del plan.

## `kind`s con reloj de evento conocido hoy

| `source` | `kind` | Reloj de evento |
|---|---|---|
| `polymarket-rtds-chainlink` | `rtds-message` | `payload.timestamp` (reloj de observación Chainlink, no el de publicación de RTDS). |
| `polymarket-gamma-clob` | `clob-message` | `timestamp` de nivel superior del mensaje (Polymarket no distingue evento/publicación en el Market Channel; por eso no hay un `publish_at_ms` separado en este contrato). |

Cualquier otro `kind` (`gamma-market-response`, y los `clob-message` cuyo `event_type` no trae
`timestamp` propio, como `new_market` en algunas respuestas observadas) queda `DEGRADED` en vez de
inventar un reloj.

## Validado contra captura real

Corriendo `prediction_replay` sobre una sesión real de ~94k eventos CLOB capturada el 2026-09-18:
93 919 `GOOD`, 39 `DEGRADED` (32 `clob-message` sin `timestamp` propio, 7
`gamma-market-response`), 0 `INVALID`. La distribución es consistente con lo observado en el RAW:
no se inventó ninguna fuente de calidad que no viniera del dato.

## Límites explícitos

- `sequence` sigue `None` para ambas fuentes actuales: no hay número de mensaje ni epoch que
  verificar. La reconstrucción de libro CLOB (ver
  [contrato de captura](RESEARCH-CAPTURE-CONTRACT.md) y el reconstructor L2 en `src/l2.rs`) no
  depende de este campo; usa sus propios snapshots `book` como puntos de resincronización.
- Este contrato es **source-local**: no fusiona eventos de distintas fuentes ni calcula lead-lag.
  Eso pertenece a la fase 3 del plan (cohortes A/B), una vez que existan collectors multi-venue.
- No se publica todavía como dataset Parquet ni con catálogo de metadatos; hoy es JSONL vía
  `prediction_replay --input --out`.
