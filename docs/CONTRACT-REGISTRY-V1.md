# Registry por contrato — `prediction-round-registry-v1`

**Estado:** implementado en `src/registry.rs`, sin red, invocable vía
`prediction_registry --gamma-input <sesión gamma> --rtds-input <sesión rtds> --out
registry.jsonl`. Cubre el punto 3 del
["orden preciso de construcción"](HANDOFF-QUANT-2026-09-18.md#orden-preciso-de-construcción):
*"Registry/labels: persistir por contrato ventana, regla TWAP, referencias y label
final; si falta evidencia, `unresolved`/`invalid`, nunca inferir."*

## Qué hace

Cruza dos sesiones RAW ya capturadas —una `polymarket-gamma-clob` (regla y ventana por
ronda) y una `polymarket-rtds-chainlink` (TWAP observado)— y produce una entrada por
ronda con la regla de resolución, la ventana exacta, y un **label auto-calculado**
(`UP`/`DOWN`/`UNRESOLVED`).

```bash
cargo run --release --bin prediction_registry -- \
  --gamma-input prediction-data/raw/<sesión>-polymarket-gamma-clob/events.jsonl \
  --rtds-input prediction-data/raw/<sesión>-polymarket-rtds-chainlink/events.jsonl \
  --out prediction-data/registry/<sesión>.jsonl
```

## Forma de una entrada

```json
{
  "schema": "prediction-round-registry-v1",
  "slug": "btc-updown-5m-1789760100",
  "question": "Bitcoin Up or Down - September 18, 3:35PM-3:40PM ET",
  "resolution_source": "https://data.chain.link/streams/btc-usd-twap-60s-streams",
  "round_start_ms": 1789760100000,
  "round_end_ms": 1789760400000,
  "twap_window_seconds": 60,
  "fee_type": "crypto_fees_v2",
  "fees_enabled": true,
  "baseline_twap": "81119.38208710155",
  "baseline_observed_at_ms": 1789760100000,
  "closing_twap": "81070.83147625328",
  "closing_observed_at_ms": 1789760400000,
  "label": "DOWN",
  "label_reason": null
}
```

## De dónde sale cada campo

- **Regla y ventana** (`question`, `resolution_source`, `twap_window_seconds`,
  `fee_type`, `fees_enabled`): del primer `gamma-market-response` visto para ese `slug`
  en la sesión Gamma — exactamente lo que Polymarket declaró para esa ronda.
- **`round_start_ms`/`round_end_ms`**: se derivan del propio slug
  (`btc-updown-5m-<segundos>`), igual que el descubrimiento en vivo
  (`prediction_clob_watch::current_slug`). No se parsean las fechas ISO-8601 de Gamma
  para esto — son redundantes con el slug y parsearlas habría exigido sumar una
  dependencia de fechas solo para esta cuenta.
- **`baseline_twap`**: el primer TWAP observado en la sesión RTDS dentro de los primeros
  5 segundos del inicio de ronda (mismo criterio que `prediction_watch::round_baseline`).
- **`closing_twap`**: el último TWAP observado dentro de los 10 segundos antes del fin de
  ronda. Una lectura más vieja que eso **no** se usa como cierre — se trataría como
  extrapolar un valor de mitad de ronda, que el contrato de captura prohíbe
  explícitamente.

## El label es una reconstrucción de investigación, no la resolución oficial

`label` aplica la misma regla que Polymarket documenta para este contrato ("Up si el
TWAP de cierre es mayor o igual al TWAP de apertura, si no Down") **sobre nuestras
propias lecturas capturadas**, no sobre el resultado que Polymarket resuelve
on-chain/en su UI. Son cosas distintas: si nuestra captura tuvo un hueco justo en el
momento de apertura o cierre de una ronda, el label queda `UNRESOLVED` en vez de
inventar un valor — nunca se sustituye una lectura faltante por la más cercana
disponible fuera de la ventana de gracia.

| `label` | Condición |
|---|---|
| `UP` | Hay baseline y cierre, y `cierre >= baseline`. |
| `DOWN` | Hay baseline y cierre, y `cierre < baseline`. |
| `UNRESOLVED` (`missing_baseline`) | No hay una lectura RTDS dentro de los primeros 5 s de la ronda. |
| `UNRESOLVED` (`missing_closing`) | No hay una lectura RTDS dentro de los últimos 10 s de la ronda. |
| `UNRESOLVED` (`unparseable_twap`) | El valor de TWAP capturado no se pudo parsear como número (no debería ocurrir con datos reales; es defensivo). |

Conectar este label con la resolución **autoritativa** de Polymarket (la que de verdad
paga el contrato) es un connector distinto, todavía no construido.

## Validado contra las tres sesiones reales capturadas el 2026-09-18

| Sesión | Rondas vistas | Con `UP`/`DOWN` | `UNRESOLVED` |
|---|---|---|---|
| `1789760073364` (gamma) + `1789760072915` (rtds) | 9 | 5 | 4 (huecos reales de captura) |
| `1789765377808` (gamma) + `1789765377808` (rtds) | 3 | 2 | 1 |
| `1789766769060` (gamma, truncada) + `1789766768578` (rtds) | 18 | 15 | 3 |

Las rondas consecutivas con evidencia completa encadenan correctamente: el `closing_twap`
de una ronda coincide con el `baseline_twap` de la siguiente (mismo valor, mismo
`observed_at_ms`), porque ambos vienen de la misma serie RTDS continua. Las rondas
marcadas `UNRESOLVED` corresponden exactamente a huecos reales de las sesiones (p. ej.
una sesión Gamma que siguió corriendo ~50 minutos después de que la sesión RTDS
correspondiente ya había terminado) — el registry no rellena esos huecos.

## Límites explícitos

- Cruza **una sesión Gamma con una sesión RTDS a la vez**; no concatena múltiples
  sesiones todavía. Para una campaña con muchas sesiones cortas, hay que correrlo por
  par y luego concatenar los JSONL resultantes.
- No valida que la sesión RTDS corresponda al mismo rango de tiempo que la sesión Gamma
  más allá de buscar timestamps que caigan dentro de cada ventana — si se cruzan sesiones
  de días distintos por error, simplemente no va a encontrar baseline/closing y todo
  quedará `UNRESOLVED` (fail-safe, no fail-silent).
- El label no reemplaza ninguna decisión de trading ni afirma edge: es un insumo de
  investigación para las fases 4 en adelante (laboratorio, validación temporal).
