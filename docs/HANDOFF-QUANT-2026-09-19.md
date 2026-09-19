# Relevo — Contrato de calidad, CLOB L2 y registry por contrato

**Fecha:** 2026-09-19
**Estado:** fase 0 (contrato de calidad + registry por contrato) y fase 2 (CLOB L2) del
[plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md) implementadas y validadas contra captura
real; sigue sin autorizarse entrenamiento, paper trading o ejecución. Siguiente pendiente: fase 3
(collectors Binance/Bybit spot/perp).
**Siguiente sesión:** leer este relevo, el [relevo anterior](HANDOFF-QUANT-2026-09-18.md), el
[contrato normalized-event-v1](NORMALIZED-EVENT-CONTRACT-V1.md), el
[contrato CLOB L2](CLOB-L2-CONTRACT-V1.md) y el
[contrato de registry](CONTRACT-REGISTRY-V1.md) antes de tocar el reconstructor o sumar fuentes.

## Qué se hizo esta sesión

1. **Se commiteó el trabajo pendiente** de la sesión anterior (consola, captura RAW, replay,
   `src/lib.rs`/`src/research.rs`, tres docs nuevos) que quedó sin commitear el 2026-09-18. `cargo
   fmt` se instaló (`rustup component add rustfmt`) y se aplicó por primera vez a todo el repo.
2. **Contrato `normalized-event-v1`** (`src/research.rs::normalize`): añade `ingest_latency_ms`,
   `publish_latency_ms`, `sequence` (reservado, hoy siempre `None`) y una clasificación de calidad
   `GOOD`/`DEGRADED`/`INVALID` con razón obligatoria cuando no es `GOOD`. Documentado en
   [NORMALIZED-EVENT-CONTRACT-V1.md](NORMALIZED-EVENT-CONTRACT-V1.md).
3. **Reconstructor CLOB L2** (`src/l2.rs`, nuevo módulo): aplica `book` (snapshot) y `price_change`
   (deltas) para mantener el libro completo por `asset_id`, con `size == "0"` eliminando el nivel.
   Invocable vía `prediction_replay --l2-out`. Documentado en
   [CLOB-L2-CONTRACT-V1.md](CLOB-L2-CONTRACT-V1.md).
4. **Se descartó un diseño de gap-detection que no sobrevivió la validación real.** El primer
   intento comparaba el mejor bid/ask reconstruido contra el que cada `price_change` declara;
   contra las tres sesiones capturadas el 2026-09-18 eso marcaba 8–56 % de los updates como "hueco",
   una tasa sin sentido. La causa (confirmada trazando mensajes concretos): un trade que toca dos
   niveles llega como mensajes WS separados que declaran el precio *final* de la operación completa,
   no el estado intermedio; en ráfagas de cientos de mensajes/segundo esa discrepancia se encadena
   varios pasos. Se reemplazó por una señal auto-verificada: `CROSSED` cuando el libro reconstruido
   tiene `best_bid >= best_ask`, imposible en un mercado sano y que no depende de confiar en nada
   que el proveedor declare. El contraste original se conserva como campo informativo
   (`declared_best_mismatch`), no sticky, sin efecto en `quality`.
5. **Se arregló una sesión interrumpida real.** Una de las tres capturas del 2026-09-18 quedó con
   su última línea truncada (proceso terminado a mitad de escritura). `prediction_replay` antes
   abortaba toda la relectura por eso; ahora tolera una última línea inválida (avisa por stderr,
   sigue) y sigue tratando cualquier otra línea inválida como error duro.
6. Se confirmó contra la documentación oficial de Polymarket (2026-09-18) que el Market Channel no
   publica número de secuencia y que el algoritmo de su campo `hash` no está documentado. Ningún
   código de este repo intenta adivinarlo.

## Verificación

- `cargo test`: **21 pruebas** pasan (14 de `src/lib.rs` — normalización + L2 —, 3 de
  `prediction_clob_watch`, 4 de `prediction_watch`).
- `cargo build --bins`: los cinco binarios compilan.
- `cargo fmt` aplicado a todo el repo.
- Replay + L2 corridos contra las tres sesiones reales capturadas el 2026-09-18
  (976 708 / 93 958 / 1 047 684 eventos RAW, ~1.6 GB total): `SYNCED` 99.2–99.8 %, `CROSSED`
  0.05–0.20 %, `AWAITING_SNAPSHOT` 0.16–0.58 %. Menos de 1 s de CPU por sesión completa. Detalle en
  [CLOB-L2-CONTRACT-V1.md](CLOB-L2-CONTRACT-V1.md#validado-contra-captura-real-2026-09-18).

## Abierto para la siguiente sesión

- **Origen de `CROSSED` (0.05–0.20 %)**: no se determinó si es un mensaje que Polymarket no envió o
  uno que nuestro propio consumidor WS (`prediction_clob_watch`) perdió bajo ráfagas de miles de
  mensajes/segundo. Antes de sumar más fuentes, instrumentar el watcher con un contador de mensajes
  recibidos vs. procesados a nivel de socket ayudaría a distinguir ambos casos.
- **Orden del plan**: con el contrato de calidad (fase 0) y CLOB L2 (fase 2) cubiertos, lo que
  sigue es el punto 3 de la ["orden preciso de construcción"](HANDOFF-QUANT-2026-09-18.md#orden-preciso-de-construcción):
  registry por contrato (ventana, regla TWAP, referencias, label final) — y solo después, el punto
  4, collectors Binance/Bybit spot/perp con su propio RAW/normalización/calidad.
- **Presupuesto de almacenamiento sigue pendiente**: las mismas tres sesiones ya ocupan ~1.6 GB en
  unas pocas horas (una sola sesión CLOB de ~13 min generó ~816 MB). Sigue sin decisión explícita
  de retención/compresión/particionado antes de una campaña larga.
- `stale_after_ms = 30_000` en `prediction_replay` es una observación inicial, no un valor
  calibrado; revisar si aparece evidencia de que es demasiado corto/largo con más sesiones.

## Límites que se mantienen sin cambio

Todos los de [HANDOFF-QUANT-2026-09-18.md](HANDOFF-QUANT-2026-09-18.md#límites-importantes) siguen
vigentes: sin wallet, credenciales, orden ni ejecución; Chainlink TWAP sigue siendo el único target
contractual; no hay señal direccional ni afirmación de edge.

## Adenda — fix de consola y literatura externa (misma fecha)

- **Fix:** la consola en modo release no podía arrancar los watchers (`START WATCHERS` fallaba con
  "no se pudo iniciar Oracle Watch"). `child()` en `src/bin/prediction_console.rs` buscaba el
  binario hermano en `target/release/`, pero su build de respaldo corría `cargo build` sin
  `--release`, que compila a `target/debug/`. Ahora detecta el perfil desde la ruta del propio
  ejecutable y pasa `--release` cuando corresponde.
- **Literatura externa revisada** (sin autorizar ninguna fase nueva): dos papers peer-reviewed sobre
  order flow/order imbalance y retornos cripto, convertidos a Markdown para referencia del repo:
  - [PAPER-DOAN-VO-2025-NET-BUYING-PRESSURE.md](PAPER-DOAN-VO-2025-NET-BUYING-PRESSURE.md) — order
    imbalance de opciones ponderado por delta de Black-Scholes, demanda direccional vs. de
    volatilidad, BTC/ETH.
  - [PAPER-ANASTASOPOULOS-2026-ORDER-FLOW-CRYPTO.md](PAPER-ANASTASOPOULOS-2026-ORDER-FLOW-CRYPTO.md) —
    world order flow (11 divisas) y su poder predictivo out-of-sample vía ML no-lineal, 84 coins.
  - [RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md](RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md) —
    síntesis: qué de esto es evidencia real (delta/order flow de opciones) vs. qué sigue sin
    evidencia académica sólida (exposición gamma/GEX de dealers), y qué haría falta capturar antes
    de poder testear cualquiera de los dos contra nuestro contrato de 5 minutos. Identifica una
    referencia ya citada en el plan (`sciencedirect.com/.../S1544612326008688`, sobre expiración de
    BTC y gamma) que sigue sin leerse — candidato natural para la próxima sesión antes de diseñar la
    fase 6 (opciones).
  - Los dos papers convertidos cubrían BTC y ETH; se recortó el contenido de ETH en ambas notas
    porque este proyecto solo cubre BTC.

## Adenda 2 — registry por contrato (misma fecha)

- **Implementado** el punto 3 del orden de construcción: `src/registry.rs` (nuevo módulo) +
  binario `prediction_registry`. Cruza una sesión RAW `polymarket-gamma-clob` (regla/ventana por
  ronda) con su sesión `polymarket-rtds-chainlink` correspondiente y produce una entrada JSONL por
  ronda con la regla de resolución y un label auto-calculado `UP`/`DOWN`/`UNRESOLVED`. El label
  aplica la regla que Polymarket documenta (`cierre >= apertura → Up`) sobre nuestras propias
  lecturas capturadas — **no es la resolución autoritativa de Polymarket/on-chain** — y queda
  `UNRESOLVED` con razón explícita (`missing_baseline`/`missing_closing`) en vez de inferir un
  valor faltante. Contrato completo en
  [CONTRACT-REGISTRY-V1.md](CONTRACT-REGISTRY-V1.md).
- Validado contra las tres sesiones pareadas del 2026-09-18: las rondas con evidencia completa
  encadenan correctamente (el cierre de una ronda es el baseline exacto de la siguiente) y las
  marcadas `UNRESOLVED` corresponden a huecos reales de captura, no a un bug.
- **Se arregló también** un bug real de la consola detectado en vivo por el usuario: cada cambio
  de ronda de 5 minutos se trataba como una falla de conexión (acumulaba el contador de
  `reconnects`, que nunca se reseteaba, y esperaba el mismo backoff exponencial que una
  desconexión real, hasta 16 s) antes de redescubrir la ronda nueva vía Gamma. Sumado al chequeo
  de rollover cada 10 s, el panel mostraba "CLOSED" hasta ~25 s de más por ronda, empeorando cuanto
  más tiempo llevaba corriendo la consola. `prediction_clob_watch.rs` ahora distingue
  `StreamExit::Rollover` (sin backoff, redetectado en ~1 s) de `StreamExit::Error` (backoff sin
  cambios).
- **cargo test**: 26 pruebas (19 de `src/lib.rs`, 3 de `prediction_clob_watch`, 4 de
  `prediction_watch`), 6 binarios compilan.
- Se preguntó por hacer push a un remoto; el usuario decidió mantenerlo local por ahora. Sigue sin
  remoto configurado — decisión pendiente de una sesión futura.
