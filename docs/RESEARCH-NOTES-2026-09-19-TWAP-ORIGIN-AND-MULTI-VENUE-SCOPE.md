# Notas de investigación — origen del TWAP y alcance real del multi-venue (2026-09-19)

**Contexto:** el usuario pidió pausar la construcción y cerrar primero la hipótesis:
¿por qué el mercado cambió de régimen de settlement?, y ¿tiene sentido expandir el
multi-venue a las ~20 fuentes que agregan herramientas tipo "Crypto Order Book Walls"
(Binance, Bybit, OKX, Bitget, Gate.io, KuCoin, HTX, MEXC, BingX, Kraken, spot+perp, más
Dydx/Hyperliquid perp)? Esta nota responde con evidencia externa (investigación web) más
verificación contra nuestros propios datos capturados — no autoriza ninguna fase nueva.

## 1. El TWAP no siempre existió: por qué cambió el régimen

**Hallazgo central: Polymarket usaba un mecanismo de settlement vulnerable hasta el
7 de agosto de 2026, cuando lo reemplazó por Chainlink TWAP Data Streams —
específicamente en respuesta a un estudio académico que documentó manipulación real por
$8.2M.**

### El mecanismo anterior (antes de agosto 2026)

Según el paper académico (ver abajo), el oracle de Chainlink que resolvía estos
contratos agregaba trades de "muchos venues centralizados y descentralizados" mediante
un procedimiento de VWAP en varias etapas, con mediana entre nodos — pero, dentro de un
horizonte de 5 minutos, el arbitraje mantiene al venue de mayor volumen (Binance) y al
precio combinado dentro de pocos basis points uno del otro. En la práctica, esto hacía
que el precio de Binance se moviera "esencialmente uno a uno" con el precio del oracle,
y que ese mecanismo fuera mucho más sensible a un empujón de corto plazo que un TWAP de
ventana completa.

### El estudio que forzó el cambio

**"Settlement Manipulation in Prediction Markets"** — David Dai (Stanford), Ruizhe Jia
(Stanford), Shihao Yu (Singapore Management University). Publicado el 30 de junio de
2026 ([arXiv:2606.31675](https://arxiv.org/abs/2606.31675),
[SSRN 7028398](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=7028398)).

Hallazgos concretos (según cobertura del propio paper):

- Analizó ~16,000 contratos BTC de 5 minutos en un período de ~2 meses tras el
  lanzamiento del contrato.
- **821 wallets de ~243,000 traders (≈0.3%)** capturaron **$8.2M** en los ciclos
  manipulados, mientras quedaban a mano (break-even) en el resto.
- **~1,600 ciclos** (el decil superior por order flow en la ventana de settlement)
  clasificados como manipulados; **6% de los ciclos** cayeron cerca del umbral, donde
  el empujón fue decisivo.
- En resultados casi-empatados, los empujones manipuladores voltearon al ganador **65%
  de las veces**, contra **41%** en trading ordinario.
- **93% de las pérdidas** se concentraron en traders retail.
- **Mecanismo:** el manipulador compra la posición binaria (Up o Down), y en los
  **últimos ~10 segundos** antes del cierre ejecuta una compra/venta agresiva de spot en
  **Binance** específicamente — porque Binance domina el volumen y, a ese horizonte, el
  arbitraje mantiene su precio pegado al del oracle a pocos bps. El market maker de
  Binance no puede distinguir esa orden de flujo ordinario no informado, así que el
  empujón se precia como si fuera normal y se traslada directo al cálculo de
  resolución.
- **Remedio que los autores respaldan con su modelo:** alargar el horizonte del
  contrato. La manipulación es marcadamente menor en los contratos de 15 minutos porque
  un empujón sostenido contra todo el resto del mercado durante más tiempo es mucho más
  caro y los resultados casi-empatados se vuelven más raros.

### La respuesta de Polymarket

Tras semanas de escrutinio por el paper, Polymarket migró los mercados de 5 y 15
minutos (y 4 horas) a settlement por **Chainlink TWAP Data Streams**, activo en mainnet
desde ~19:55 UTC del 31 de julio de 2026 y en vivo para los mercados desde el **7 de
agosto de 2026, 00:00 UTC**
([Genfinity](https://genfinity.io/2026/08/12/chainlink-twap-data-streams-polymarket-crypto-markets/),
[Coinpaprika](https://coinpaprika.com/news/chainlink-twap-feeds-live-polymarket-close/)).
La lógica del cambio: con un TWAP de ventana completa, un empujón debe sostenerse
durante *toda* la ventana contra todo el que lo contra-opera, no solo durante un
instante — la estrategia documentada en el paper deja de ser viable tal cual.

### Discrepancia real encontrada al verificar contra nuestros propios datos

La cobertura de prensa (Genfinity, y el resumen de búsqueda inicial) dice: **"Five-minute
markets use a 30-second averaging window."** Nuestras propias 13 rondas capturadas en
RAW real el 2026-09-18 y 2026-09-19 —de sesiones distintas, en horas distintas— dicen
unánimemente **`twapLookbackSeconds: 60`** para `btc-updown-5m-*`, nunca 30. Ver
`prediction-data/raw/*/events.jsonl`, evento `gamma-market-response`.

No se pudo confirmar la razón exacta de la discrepancia contra la documentación oficial
(la página de Polymarket sobre Chainlink TWAP describe la implementación técnica, no
los parámetros de mercados específicos). Puede ser un error de la prensa al reportar el
diseño original, o un ajuste posterior al lanzamiento. **Para este proyecto, el dato que
manda es el que Gamma declara en vivo por ronda (nuestra propia captura), no lo que
reporta la prensa** — exactamente la disciplina que ya seguíamos, ahora con un ejemplo
concreto de por qué importa.

## 2. Multi-venue: qué es realmente lo que herramientas como "Crypto Order Book Walls" hacen

Herramientas de agregación de libro (TapeSurf, CoinLobster, Amberdata, Bookmap
Multibook) combinan bids/asks de 10-20+ exchanges (spot y perp) en una sola vista de
profundidad denominada en dólares. Su valor documentado es principalmente **visual y de
detección de paredes de liquidez** (walls) que podrían estar ocultas si se mira un solo
exchange — no hay evidencia de que agregar más de un puñado de venues aporte una señal
*predictiva* adicional más allá de lo que ya capturan los 2-3 venues dominantes, para
horizontes cortos como el nuestro.

Un hallazgo de la literatura de microestructura citado en la búsqueda: la investigación
académica sobre price discovery en cripto usa metodologías como el **Hasbrouck
Information Share** o la **descomposición de Gonzalo-Granger** específicamente para
medir *cuánto* aporta cada venue al descubrimiento de precio — no se asume que todos
aporten igual, se mide.

## 3. Cómo esto cambia (o refina) nuestra hipótesis de multi-venue

**Antes de esta investigación:** la justificación de multi-venue era genérica —
"formación de precio independiente, nowcasting, microestructura".

**Después de esta investigación, la justificación es mecánica y específica:**

1. **Binance domina la ponderación efectiva del oracle en este horizonte.** El propio
   paper lo confirma empíricamente: dentro de 5 minutos, el arbitraje mantiene el precio
   de Binance pegado al del oracle combinado a pocos bps, y se mueve "uno a uno" con él.
   Esto significa que observar Binance en tiempo real **no es una fuente más entre
   varias equivalentes — es la fuente con mayor peso causal conocido** sobre lo que el
   TWAP va a promediar.
2. **Bybit es la segunda fuente razonable, no la única alternativa obvia entre 20.**
   El paper no estudia Bybit específicamente, pero su misma lógica de arbitraje
   (cualquier venue líquido de alto volumen queda pegado a los demás en <5 min) sugiere
   que agregar OKX, Bitget, Gate.io, KuCoin, HTX, MEXC, BingX, Kraken tiene **valor
   marginal decreciente** para *este* mecanismo específico, salvo que se demuestre lo
   contrario con datos (ablación, no supuesto — exactamente el método que ya definimos).
3. **Aparece un caso de uso nuevo, no contemplado antes: detección de manipulación
   residual.** El TWAP de 60s reduce la manipulación pero el propio paper dice
   "atenuada", no "eliminada". Un empujón sostenido en Binance durante toda la ventana
   de 60s sigue siendo teóricamente posible para un actor con más capital. Nuestra
   propia captura de order flow de Binance en tiempo real es exactamente el insumo para
   **detectar** (no para replicar) un patrón de flujo unidireccional anómalo sostenido
   cerca del cierre de ronda — un flag de calidad/anomalía, no una señal direccional
   para apostar. Esto es coherente con el espíritu read-only y de investigación del
   proyecto: sirve para *desconfiar* de una ronda con evidencia de posible manipulación,
   no para copiar la estrategia de los manipuladores.
4. **El arbitraje cross-venue como tal no es la fuente de edge.** La búsqueda general
   confirma lo esperado: las diferencias de precio entre exchanges se cierran por
   arbitraje en milisegundos; no es ahí donde está la información aprovechable para
   nuestro caso. Lo aprovechable es la **ventana de tiempo entre que el precio se forma
   en Binance/Bybit y que Chainlink lo reporta oficialmente** (nowcasting), y
   potencialmente la **detección de flujo anómalo** — no un arbitraje entre exchanges
   per se.

## 4. Recomendación concreta para la decisión de "cuántos exchanges"

No expandir a los ~20 venues del estilo "Order Book Walls" todavía. En su lugar:

1. Con Binance y Bybit ya capturados (fase 3 actual), diseñar el primer estudio
   específicamente alrededor de **la ventana de settlement de 60s**: ¿el order flow de
   Binance en esos últimos 60s tiene poder predictivo/anómalo medible sobre el label
   final de la ronda (nuestro propio registry)? Esto es la pregunta que el paper ya
   demostró que importaba bajo el régimen viejo — hay que remedirla bajo el régimen
   TWAP nuevo, con datos propios.
2. Solo si Binance+Bybit no explican suficiente varianza, o si se detecta que la
   manipulación se desplazó a otro venue (algo plausible: si el paper mostró que
   Binance era el vector bajo el mecanismo viejo, actores sofisticados podrían migrar de
   venue bajo el nuevo régimen), evaluar sumar un tercer venue con evidencia, no por
   completar la lista.
3. Esto es literalmente el "orden preciso de construcción" que ya teníamos —esta
   investigación no lo cambia, lo confirma con un motivo mecánico concreto en vez de uno
   genérico.

## Fuentes

- [Settlement Manipulation in Prediction Markets (arXiv:2606.31675)](https://arxiv.org/abs/2606.31675) — Dai, Jia, Yu (Stanford/SMU), 2026-06-30.
- [SSRN 7028398](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=7028398)
- [Stanford Study Finds Signs of Bitcoin Market Manipulation on Polymarket — Yahoo Finance](https://finance.yahoo.com/markets/crypto/articles/stanford-study-finds-signs-bitcoin-134715897.html)
- [Stanford Study: Polymarket's Bitcoin Bets Were Rigged in the Final 10 Seconds — Bitcoin.com](https://news.bitcoin.com/stanford-study-polymarket-bitcoin-manipulation/)
- [Chainlink TWAP Feeds Go Live as Polymarket Moves to Close a Manipulation Window — Coinpaprika](https://coinpaprika.com/news/chainlink-twap-feeds-live-polymarket-close/)
- [Chainlink TWAP Data Streams Now Settle Polymarket's 5- and 15-Minute Crypto Markets — Genfinity](https://genfinity.io/2026/08/12/chainlink-twap-data-streams-polymarket-crypto-markets/)
- [Chainlink TWAP Prices — Polymarket Documentation](https://docs.polymarket.com/market-data/chainlink-twap)
- Verificación propia: `prediction-data/raw/*-polymarket-gamma-clob/events.jsonl`, evento `gamma-market-response`, campo `cryptoMarketConfig.twapLookbackSeconds` — 13/13 rondas capturadas dicen 60, no 30.

## Qué queda abierto (para seguir investigando antes de construir, tal como pidió el usuario)

- Confirmar directamente con Polymarket/Chainlink por qué la ventana real es 60s y no
  30s como reporta la prensa — no resuelto con las fuentes públicas disponibles.
- No se investigó todavía si existe evidencia (post-migración a TWAP, es decir después
  del 2026-08-07) de que la manipulación se desplazó a otro venue o se adaptó al nuevo
  régimen — sería el dato más directamente accionable para decidir si Bybit (u otro
  venue) merece prioridad sobre simplemente profundizar en Binance.
- No se investigó la composición exacta de nodos/exchanges que Chainlink pondera
  internamente en su agregación — sigue siendo opaca, como ya establecía
  `ARCHITECTURE.md`.
