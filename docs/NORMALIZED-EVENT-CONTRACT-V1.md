# Contrato `normalized-event-v1`

**Estado:** implementado en `src/research.rs::normalize` y `prediction_replay`; read-only,
source-local, sin red. Cubre la fase 0 del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md)
("contrato de calidad") y ya cubre las seis fuentes actuales (Polymarket RTDS/CLOB y el
multi-venue mínimo de Binance/Bybit — ver [MULTI-VENUE-CONTRACT-V1.md](MULTI-VENUE-CONTRACT-V1.md));
queda abierto a nuevas fuentes sin cambiar su forma.

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

## Reglas de calidad (deterministas, con una única tolerancia justificada por dato real)

| `quality` | Condición | `quality_reason` |
|---|---|---|
| `GOOD` | `event_at_ms` presente y `publish_latency_ms >= -250 ms`. | — |
| `DEGRADED` | El `kind` no tiene un reloj de evento conocido (p. ej. `gamma-market-response`, `new_market`, `tick_size_change`, `bookTicker` de Binance, un ack de suscripción de Bybit). | `unrecognized_kind` / `no_event_clock_declared` |
| `INVALID` | `event_at_ms` más de 250 ms posterior a `received_at_ms`: reloj imposible, no explicable por desfase normal entre máquinas. | `event_after_receive` |

**Por qué 250 ms y no 0:** la primera versión de esta regla marcaba `INVALID` cualquier
`publish_latency_ms` negativo, incluido -1 ms. Validada contra una sesión real de 200 s
capturada el 2026-09-19, esa regla clasificó **81 % de los eventos `clob-message` de esa sesión
como `INVALID`** — pero los 65 991 casos tenían un desfase de entre -1 ms y -64 ms, exactamente
el rango de desfase de reloj esperable entre dos máquinas distintas (ver
`tolera_un_desfase_de_reloj_pequeno_entre_maquinas` en `src/research.rs`). Era un falso positivo
masivo por una regla demasiado estricta, no un problema del dato. El margen de 250 ms deja holgura
sobre el máximo observado (64 ms) sin dejar de detectar un reloj genuinamente imposible (segundos
de diferencia).

`DEGRADED` no descarta el payload: se conserva completo en `payload` para que una fase posterior
decida si lo usa. Un evento `DEGRADED` o `INVALID` no debe alimentar features ni labels sin decisión
explícita de la fase 5 del plan.

## `kind`s con reloj de evento conocido hoy

| `source` | `kind` | Reloj de evento |
|---|---|---|
| `polymarket-rtds-chainlink` | `rtds-message` | `payload.timestamp` (reloj de observación Chainlink, no el de publicación de RTDS). |
| `polymarket-gamma-clob` | `clob-message` | `timestamp` de nivel superior del mensaje (Polymarket no distingue evento/publicación en el Market Channel; por eso no hay un `publish_at_ms` separado en este contrato). |
| `binance-spot`, `binance-perp` | `binance-*-message` | Solo `aggTrade` (campo `T`, hora del trade) y `markPrice`/`markPrice@1s` (campo `E`, hora del evento — `T` ahí es el *próximo funding*, no el reloj del evento). `bookTicker` no declara ningún reloj. |
| `bybit-spot`, `bybit-perp` | `bybit-*-message` | `ts` de nivel superior, presente en todos los topics de mercado (`orderbook.1`, `publicTrade`, `tickers`); los mensajes de control (ack de suscripción, pong) no lo traen. |

Cualquier otro `kind` (`gamma-market-response`, y los `clob-message` cuyo `event_type` no trae
`timestamp` propio, como `new_market` en algunas respuestas observadas) queda `DEGRADED` en vez de
inventar un reloj.

## Validado contra captura real

- **2026-09-18**, ~94k eventos CLOB de una sesión: 93 919 `GOOD`, 39 `DEGRADED` (32 `clob-message`
  sin `timestamp` propio, 7 `gamma-market-response`), 0 `INVALID`.
- **2026-09-19**, sesión de 200 s con los seis venues corriendo a la vez (la que reveló la
  necesidad de la tolerancia de 250 ms): tras el fix, `polymarket-gamma-clob` quedó en 100 %
  `GOOD`/`DEGRADED` (0 `INVALID`, sobre 81 164 eventos); `binance-spot` en 8.9 % `GOOD`
  (`aggTrade`) / 91.1 % `DEGRADED` (`bookTicker`, sin reloj); `bybit-spot`/`bybit-perp` en
  ~98-99.6 % `GOOD`. `binance-perp` quedó 100 % `DEGRADED` porque en este entorno de red solo
  llegó `bookTicker` — ver la limitación documentada en
  [MULTI-VENUE-CONTRACT-V1.md](MULTI-VENUE-CONTRACT-V1.md).

## Límites explícitos

- `sequence` sigue `None` para todas las fuentes actuales: ninguna declara un número de mensaje o
  epoch verificable. La reconstrucción de libro CLOB (ver
  [contrato de captura](RESEARCH-CAPTURE-CONTRACT.md) y el reconstructor L2 en `src/l2.rs`) no
  depende de este campo; usa sus propios snapshots `book` como puntos de resincronización.
- Este contrato es **source-local**: no fusiona eventos de distintas fuentes ni calcula lead-lag.
  Eso sigue siendo la puerta de la fase 3 del plan (cohortes A/B), ahora que ya existen los
  collectors multi-venue (ver [MULTI-VENUE-CONTRACT-V1.md](MULTI-VENUE-CONTRACT-V1.md)) — falta el
  estudio descriptivo en sí, que necesita una campaña de captura más larga que las pruebas de
  segundos usadas para validar este contrato.
- No se publica todavía como dataset Parquet ni con catálogo de metadatos; hoy es JSONL vía
  `prediction_replay --input --out`.
- La tolerancia de 250 ms es una observación inicial sobre una sola sesión de 200 s, no un valor
  calibrado con más máquinas, redes o duraciones; revisar si aparece evidencia de que es
  demasiado corta o demasiado holgada con más capturas.
