# Contrato `prediction-clob-l2-update-v1`

**Estado:** implementado en `src/l2.rs`, sin red, invocable vía
`prediction_replay --input events.jsonl --out normalized.jsonl --l2-out l2.jsonl`. Cubre la fase 2
del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md) ("CLOB L2: snapshot inicial, deltas,
secuencia, gaps, resync y replay determinista") para el Market Channel de Polymarket.

## Lo que Polymarket expone y lo que no

Confirmado contra la documentación oficial el 2026-09-18: el Market Channel **no** publica un
número de secuencia, y el algoritmo del campo `hash` que acompaña a `book` y `price_change` no está
documentado públicamente. Este contrato nunca finge verificar ese `hash`; en vez de eso reconstruye
el libro completo por `asset_id` a partir de dos tipos de mensaje ya capturados:

- `book`: snapshot completo (bids/asks) — único punto de resincronización de confianza.
- `price_change`: uno o más `{asset_id, side, price, size}`; `size == "0"` elimina el nivel, otro
  valor lo reemplaza (no lo suma). Un solo mensaje puede tocar varios `asset_id` a la vez (p. ej.
  Up y Down de la misma ronda, porque son complementarios).

## Forma del update

```json
{
  "schema": "prediction-clob-l2-update-v1",
  "asset_id": "1073...",
  "event_at_ms": 1789760074559,
  "received_at_ms": 1789760074561,
  "best_bid": "0.73",
  "best_ask": "0.74",
  "spread": "0.010000",
  "bid_levels": 42,
  "ask_levels": 57,
  "quality": "SYNCED",
  "quality_reason": null,
  "declared_best_mismatch": false
}
```

Se emite un `L2Update` por cada `asset_id` que un mensaje haya tocado. Sin un `book` previo, la
profundidad acumulada de deltas sueltos no se reporta como si fuera confiable: `best_bid`,
`best_ask`, `spread`, `bid_levels` y `ask_levels` quedan en `None`/`0`.

## Calidad: dos señales verificables, no un hash que no se puede probar

| `quality` | Condición | `quality_reason` |
|---|---|---|
| `AWAITING_SNAPSHOT` | No llegó un `book` para este asset en esta reconstrucción. | `no_snapshot_yet` |
| `SYNCED` | Hay un `book` de base y el libro reconstruido no cruza. | — |
| `STALE` | Este update llegó tras un silencio mayor al esperado (`stale_after_ms`, hoy 30 s en `prediction_replay`) desde el último evento del mismo asset. Se limpia con el siguiente evento; no implica libro corrupto, solo baja actividad. | `idle_before_update` |
| `CROSSED` | El libro reconstruido tiene `best_bid >= best_ask`: imposible en un mercado sano. Persiste hasta el siguiente `book`, el único resync de confianza. | `crossed_book` |

`CROSSED` es **auto-verificado**: no depende de comparar contra nada que el proveedor declare, solo
de la consistencia interna del libro que reconstruimos nosotros mismos.

## El intento descartado: comparar contra el `best_bid`/`best_ask` declarado

La primera versión de este reconstructor escalaba a "hueco" cuando el mejor precio reconstruido
localmente no coincidía con el `best_bid`/`best_ask` que cada `price_change` declara. Validado
contra las tres sesiones reales capturadas el 2026-09-18, esa señal marcó entre 8 % y 56 % de los
updates como huecos — una tasa que no se correspondía con nada real.

La causa, confirmada trazando mensajes concretos del RAW: una operación que toca dos niveles (p.
ej. un trade que vacía un nivel de venta y agrega uno de compra) llega como mensajes WS separados
con el mismo timestamp, y cada uno declara el mejor precio **final** de la operación completa, no
el estado tras esa única línea. Durante ráfagas de alta actividad (cientos de mensajes por segundo)
esa discrepancia transitoria puede encadenarse varios pasos seguidos antes de resolverse sola. Como
el estado quedaba pegado hasta el siguiente `book` (~2 % de los mensajes), una sola ráfaga
contaminaba miles de updates posteriores.

Ese contraste se conserva como campo informativo (`declared_best_mismatch: Option<bool>`, no
sticky, sin efecto en `quality`) para quien quiera estudiar el patrón de batching de Polymarket más
adelante, pero no se usa como veredicto de integridad. Ver `no_confunde_un_trade_partido_con_un_hueco` y `un_trade_partido_en_dos_mensajes_no_degrada_la_calidad` en `src/l2.rs` para el caso
real reproducido en un test.

## Validado contra captura real (2026-09-18)

| Sesión | Updates | `SYNCED` | `CROSSED` | `AWAITING_SNAPSHOT` |
|---|---|---|---|---|
| `1789760073364-polymarket-gamma-clob` (976 708 eventos RAW) | 1 895 076 | 99.22 % | 0.20 % | 0.58 % |
| `1789765377808-polymarket-gamma-clob` (93 958 eventos RAW) | 180 362 | 99.49 % | 0.05 % | 0.46 % |
| `1789766769060-polymarket-gamma-clob` (1 047 684 eventos RAW, sesión interrumpida) | 2 002 115 | 99.77 % | 0.08 % | 0.16 % |

Menos de 1 segundo de CPU para reconstruir cada sesión completa. La tasa de `CROSSED` (0.05–0.20 %)
es la primera medición real de inconsistencia genuina del libro reconstruido; no se ha determinado
todavía si el origen es un mensaje perdido por Polymarket, un drop en nuestro propio consumidor WS
bajo ráfagas de miles de mensajes por segundo, o ambos — queda abierto para la fase 3 del plan
(multi-venue) y para instrumentar el propio `prediction_clob_watch` con un contador de mensajes
perdidos a nivel de socket.

## Robustez de replay ante sesiones interrumpidas

La sesión `1789766769060-polymarket-gamma-clob` terminó con su última línea truncada a mitad de
escritura (el proceso se detuvo sin cerrar limpio). `prediction_replay` y `replay_l2` toleran esto:
si solo la **última** línea de un `events.jsonl` no es JSON válido, se avisa por stderr y se ignora
esa línea en vez de abortar toda la relectura; cualquier otra línea inválida en medio del archivo
sigue siendo un error duro, porque ahí sí podría ser corrupción real. Ver
`tolera_una_ultima_linea_truncada_por_una_sesion_interrumpida` en `src/research.rs`.

## Límites explícitos

- No hay verificación criptográfica de integridad: Polymarket no documenta el algoritmo de su
  `hash`, y no se intentó adivinarlo. `CROSSED` es la única señal de hueco con evidencia propia.
  `STALE` y `AWAITING_SNAPSHOT` describen ausencia/silencio, no corrupción.
- La reconstrucción es **por sesión**: no persiste estado entre archivos ni asume relación entre
  `asset_id` de rondas distintas.
- No calcula todavía métricas de profundidad (OFI, queue imbalance, slope) mencionadas en la
  sección 5.2 del plan; expone la forma del libro (`bid_levels`/`ask_levels`, mejor precio, spread)
  para que esa capa se construya encima con su propia validación.
- `stale_after_ms` (30 s) es una observación inicial sobre sesiones cortas, no un valor calibrado
  contra campañas largas ni distintos regímenes de actividad.
