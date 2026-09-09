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
