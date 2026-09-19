# Arquitectura inicial

```text
Polymarket RTDS ──→ Oracle Watch ──→ prediction-state/prediction-watch.json
Gamma + CLOB WSS ──→ CLOB Watch ──→ prediction-state/prediction-clob-watch.json
                                           ↓
                           Nakamotito Prediction Console (pendiente)
```

El estado IPC es efímero y vence a los diez segundos. Un consumidor debe descartarlo tras ese
vencimiento. Cada dato conserva el reloj de evento/publicación, el de recepción y la salud del
stream; no se sustituye un feed de resolución por una cotización spot.

## Límites

- Read-only; no wallet, claves, órdenes ni ejecución.
- Sin persistencia histórica, replay o afirmaciones de edge.
- Runtime, estado, UI y CI propios; no dependencia de `nakamotito` (Trading).

## Dirección de arquitectura aprobada — aún no implementada

Prediction Console se orienta a **Settlement Intelligence multi-venue**. No toma a Binance ni a
BNB Chain como centro o proxy suficiente de un oracle: para cada mercado debe registrar la regla
de resolución, feed ID, ventana, relojes, red/mecanismo de entrega y costes, y contrastar el
reporte de referencia real con una matriz propia de spot, perps y —cuando aporte información
medible— liquidez on-chain.

La futura capa de investigación conserva cuatro planos separados: formación de precio, oracle,
entrega/resolución y CLOB de predicción. El estado global `G(t)` mantiene por fuente su semántica,
relojes, latencia y calidad; no es un promedio fijo. El nowcast del reporte `Ĉ(t)` y el forecast
de resolución `P(UP)` son modelos diferentes. Ninguno se implementará ni se considerará edge sin
captura autorizada, validación temporal y costes/fills realistas.

Canon transversal: `nakamotito-knowledge/02-Arquitectura/Oracle-y-Settlement-Intelligence-Multi-Venue.md`.
