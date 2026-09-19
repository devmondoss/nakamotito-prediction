# Multi-venue mínimo — Binance y Bybit spot/perp

**Estado:** implementado, cuatro binarios read-only y efímeros. Cubre el punto 4 del
["orden preciso de construcción"](HANDOFF-QUANT-2026-09-18.md#orden-preciso-de-construcción)
("collectors separados de Binance spot/perp y Bybit spot/perp, con RAW, normalización y
calidad propios") y arranca la fase 3 del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md)
("Cohortes A/B: Chainlink+CLOB+Binance+Bybit sincronizados").

## Por qué esto no compite con el TWAP de Chainlink

Chainlink publica el mismo TWAP en ~30-50 blockchains — eso es la capa de **entrega**,
no dice qué exchanges componen internamente el feed (ver `docs/ARCHITECTURE.md`). Estos
cuatro collectors no intentan reproducir ni adivinar esa fórmula interna. Sirven para:
nowcasting (`Ĉ(t)`) del reporte antes de que Chainlink lo publique, features de
microestructura para `q(t)`, y una señal de divergencia/calidad independiente. Ver
[RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md](RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md)
para la discusión completa de esta distinción.

## Los cuatro collectors

| Binario | Venue | WS | Qué observa |
|---|---|---|---|
| `prediction_binance_spot_watch` | Binance spot BTCUSDT | `stream.binance.com` combinado `bookTicker`+`aggTrade` | mejor bid/ask, spread, último trade con lado (agresor) |
| `prediction_binance_perp_watch` | Binance perp USD-M BTCUSDT | `fstream.binance.com` combinado `bookTicker`+`aggTrade`+`markPrice@1s` | lo mismo + mark price, index price, funding rate |
| `prediction_bybit_spot_watch` | Bybit spot BTCUSDT | `stream.bybit.com/v5/public/spot`, canales `orderbook.1`+`publicTrade` | mejor bid/ask, spread, último trade con lado declarado |
| `prediction_bybit_perp_watch` | Bybit perp lineal BTCUSDT | `stream.bybit.com/v5/public/linear`, + canal `tickers` | lo mismo + mark price, funding rate, open interest en USD |

Cada uno se ejecuta igual que los watchers existentes:

```bash
cargo run --release --bin prediction_binance_spot_watch -- --out prediction-state --record prediction-data
cargo run --release --bin prediction_binance_perp_watch -- --out prediction-state --record prediction-data
cargo run --release --bin prediction_bybit_spot_watch -- --out prediction-state --record prediction-data
cargo run --release --bin prediction_bybit_perp_watch -- --out prediction-state --record prediction-data
```

Cada sesión crea su propio `prediction-data/raw/<timestamp>-<source>/` (`source` es
`binance-spot`, `binance-perp`, `bybit-spot` o `bybit-perp`), reutilizando el mismo
`ResearchWriter` append-only que ya usan los watchers de Polymarket — mismo contrato de
captura, misma tolerancia a sesiones interrumpidas en el replay.

## Plomería compartida

`src/watch_support.rs` (nuevo) centraliza lo que ya se repetía entre watchers: el
backoff exponencial acotado (`retry_delay`, antes duplicado solo en
`prediction_clob_watch`) y la escritura atómica de snapshots (`write_snapshot_atomic`,
archivo temporal + rename). `prediction_clob_watch` se migró a usar el `retry_delay`
compartido en vez de su copia local. El parseo de cada venue queda intencionalmente
**sin** abstraer entre sí: Binance y Bybit tienen formas de mensaje completamente
distintas, y forzar una interfaz común ahora sería la abstracción prematura que este
proyecto evita.

## Detalles de parseo por venue

- **Binance** (`aggTrade`): el lado del agresor se deriva del flag `m` (`true` = el
  comprador fue el maker ⇒ el trade fue iniciado por un vendedor). Esto no es un dato
  que Binance declare directamente como "side"; es la convención documentada de su API.
- **Bybit** (`publicTrade`): el lado (`S`: `Buy`/`Sell`) viene declarado directamente por
  Bybit — se conserva tal cual, sin inferencia.
- **Bybit `tickers`**: es un canal acumulativo — manda un `snapshot` inicial y luego
  `delta`s que solo traen los campos que cambiaron. El merge (`merge_ticker`) actualiza
  únicamente los campos presentes en cada mensaje; un campo ausente nunca se interpreta
  como "vacío", solo como "sin cambio en este mensaje".
- **`orderbook.1` de Bybit**: es profundidad 1 (solo el mejor nivel), así que cada
  mensaje ya es el estado completo del top-of-book — no hace falta la reconstrucción L2
  que sí requirió el CLOB de Polymarket (ver `docs/CLOB-L2-CONTRACT-V1.md`).

## Validado contra los venues reales (2026-09-19)

Los cuatro binarios se probaron en vivo contra los endpoints públicos reales (no solo
con fixtures):

| Collector | Resultado |
|---|---|
| Binance spot | Conectó en ~1 s; `bookTicker` y `aggTrade` fluyendo, ~90 eventos/s capturados en RAW. |
| Bybit spot | Conectó en ~1.4 s; `orderbook.1` y `publicTrade` fluyendo con lado declarado. |
| Bybit perp | Conectó en ~1.5 s; `orderbook.1`, `publicTrade` y `tickers` (mark price, funding, OI) fluyendo; el merge acumulativo de `tickers` se comportó como se esperaba. |
| Binance perp | `bookTicker` fluyó con normalidad; **`aggTrade` y `markPrice@1s` no entregaron ningún mensaje en 15 s de prueba**, incluso probando cada stream por separado con un cliente independiente (Node, no el binario Rust). La conexión queda abierta, sin error ni cierre — no es una excepción capturable. |

### La limitación de Binance perp es del entorno, no del código

El parseo de `aggTrade`/`markPrice` en `prediction_binance_perp_watch` está probado con
fixtures literales que siguen el esquema documentado de Binance (idéntico al usado en el
watcher de spot, que sí valida en vivo) — el código no es lo que falló. El patrón
observado (spot funciona completo, perp entrega `bookTicker` pero no trade/mark price)
es consistente con una restricción geográfica/de cumplimiento de Binance sobre datos de
derivados según el origen de red, no con un bug de esta implementación. **Queda
pendiente de re-validar `prediction_binance_perp_watch` desde una red distinta** (p. ej.
la máquina del usuario) antes de asumir que captura funding real en producción.

## Límites explícitos

- Sin red-only violado en ningún punto: cuatro procesos read-only, sin wallet, sin
  claves, sin órdenes.
- No hay todavía normalización cruzada entre estos cuatro venues y Chainlink/CLOB — eso
  es la puerta de la fase 3 ("lead-lag descriptivo sin leakage") y sigue pendiente.
- No hay libro L2 de profundidad para ninguno de los dos exchanges (solo top-of-book);
  la sección 5.2 del plan lo deja para cuando haya prueba de que aporta sobre lo ya
  capturado.
- Ningún dato de estos collectors alimenta todavía features ni modelos — es únicamente
  captura, siguiendo el mismo orden que ya se siguió con Polymarket (capturar y validar
  antes de calcular).
