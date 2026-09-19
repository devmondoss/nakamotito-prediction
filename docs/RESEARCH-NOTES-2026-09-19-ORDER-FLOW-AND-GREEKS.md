# Notas de investigación — order flow, griegas y gamma (2026-09-19)

**Contexto:** un quant trader le comentó al usuario que las griegas y la gamma podrían
servir para predecir el precio. En la misma sesión se compartieron dos papers
académicos sobre order imbalance/order flow y retornos cripto. Esta nota resume qué
dicen esos papers, qué NO dicen sobre gamma específicamente, el estado real de la
evidencia sobre exposición gamma (GEX), y cómo se conecta —o no— con nuestro contrato
BTC Up/Down 5m de Polymarket. Es investigación, no implementación: no autoriza ninguna
fase nueva del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md).

## Los dos papers convertidos

- [Doan & Vo (2025) — Net buying pressure entre derivados y spot](PAPER-DOAN-VO-2025-NET-BUYING-PRESSURE.md)
- [Anastasopoulos et al. (2026) — Order flow y retornos de criptomonedas](PAPER-ANASTASOPOULOS-2026-ORDER-FLOW-CRYPTO.md)

## Qué SÍ es evidencia real de "griegas prediciendo precio"

Doan & Vo usan el **delta de Black-Scholes** —una griega— de dos formas: para
clasificar moneyness (ATM/OTM/DOTM/ITM/DITM) y para ponderar el volumen buy/sell al
construir el order imbalance de opciones y su descomposición en demanda
**direccional** vs. **de volatilidad**. Es evidencia peer-reviewed real de que:

- la demanda direccional en opciones OTM predice retornos futuros de BTC (positivo,
  significativo al 1 % en varias especificaciones);
- la demanda de volatilidad predice mayor volatilidad implícita/realizada futura, de
  forma consistente a través de moneyness y de BTC/ETH;
- el efecto es más fuerte cuando el mercado subyacente está líquido (BTC) o ilíquido
  (ETH, para volatilidad) — el canal de liquidez importa.

Esto **sí** justifica la intuición del trader de que "las griegas ayudan a leer el
precio" — pero specifically vía **delta** como ponderador de order flow, no vía
**gamma**.

## Qué NO cubren estos papers: exposición gamma (GEX)

Ninguno de los dos papers calcula ni menciona gamma exposure (GEX) de dealers. La
"gamma" a la que probablemente se refería el trader es un concepto distinto:

### El mecanismo teórico de GEX

Cuando un market maker vende una opción, típicamente cubre su exposición direccional
(delta-hedging) comprando o vendiendo el subyacente. La **gamma** de esa posición
determina cuánto tiene que reajustar ese hedge cuando el precio se mueve:

- Si los dealers están **netos largos gamma** (común cuando el flujo público compra
  opciones que los dealers venden), su hedge los obliga a **vender en rallies y comprar
  en caídas** → esto amortigua el movimiento (mean-reversion), y puede generar "pinning"
  del precio cerca de strikes con mucho open interest al vencimiento.
- Si están **netos cortos gamma**, el hedge es en la dirección contraria al movimiento
  → **amplifica** rallies y caídas (momentum).

Esta es la lógica detrás de indicadores de "GEX" popularizados por proveedores como
SqueezeMetrics y sus derivados (SpotGamma y similares), principalmente sobre SPX/opciones
de acciones.

### Por qué esto no es tan sólido como parece

1. **GEX es una estimación, no una observación.** El open interest público no dice quién
   compró y quién vendió cada contrato, así que cualquier cálculo de GEX asume una
   convención (p. ej. "todo el volumen de calls es comprado por el público, todo el de
   puts es vendido por el público") que puede no reflejar la posición real de los
   dealers. Nuestro propio plan ya lo marca así:
   > "`GEX` es una estimación de régimen con supuestos; no es inventario de dealers
   > observado. [...] No se publica `dealer_position_observed`: OI no revela quién
   > está largo/corto ni el hedge real del dealer." — `QUANT-SETTLEMENT-PLAN-2026-09-18.md`,
   > secciones 5.3 y 10.
2. **La literatura peer-reviewed sobre GEX específicamente en cripto es escasa.** La
   mayoría de la evidencia pública sobre GEX viene de proveedores comerciales (no
   revisados por pares) y de estudios en SPX/acciones. Ninguno de los dos papers de
   esta sesión mide GEX; solo miden order imbalance ponderado por delta.
3. **El propio plan ya tenía una referencia pendiente de leer sobre esto exacto:**
   > "Estudio de expiración BTC y gamma" —
   > [sciencedirect.com/science/article/pii/S1544612326008688](https://www.sciencedirect.com/science/article/pii/S1544612326008688)
   listado en `QUANT-SETTLEMENT-PLAN-2026-09-18.md` como referencia de diseño. **No se ha
   leído ni resumido todavía** — es el candidato natural para responder con evidencia
   real si gamma/GEX importa específicamente para BTC, antes de construir nada. Próximo
   paso recomendado: pedir ese PDF y convertirlo igual que estos dos.

## La cadena causal que haría falta para nuestro contrato específico

Nuestro contrato no es una opción — es un binario de Polymarket que resuelve contra el
**TWAP de Chainlink de BTC/USD**, sin mecanismo de dealer-hedging propio. Para que
"gamma" importe para *nuestra* predicción, la cadena tendría que ser indirecta:

```
Dealers de opciones BTC (Deribit) rebalancean su hedge
        → ese flujo mueve el spot/perp de BTC en CEXs
                → ese movimiento se refleja en el TWAP de Chainlink
                        → eso determina el resultado Up/Down de la ronda de 5 min
```

Cada flecha de esa cadena es una hipótesis separada que requiere su propia validación
temporal — exactamente el tipo de generalización que el plan prohíbe asumir sin
evidencia ("no se generaliza tarifa, oracle o resolución entre venues", sección 10). Es
plausible en teoría, pero no hay nada en el proyecto ni en estos dos papers que la
confirme para ventanas de 5 minutos específicamente. Los dos papers convertidos
trabajan a frecuencia horaria/diaria — un orden de magnitud más lento que nuestra
ronda.

## Qué haría falta capturar para poder testear esto con evidencia propia

Nada de esto está autorizado todavía; es el inventario de lo que faltaría, en el orden
que ya define el plan:

1. **Registry por contrato** (fase 0, pendiente): antes que nada, formalizar qué
   contratos/ventanas se están observando — ya identificado como próximo paso en
   [HANDOFF-QUANT-2026-09-19.md](HANDOFF-QUANT-2026-09-19.md).
2. **Collectors spot/perp de Binance/Bybit** (fase 3, no iniciada): sin esto no hay ni
   siquiera el primer eslabón de la cadena (el movimiento de spot/perp) capturado con
   calidad propia.
3. **Collector de opciones Deribit** (fase 6, no iniciada): OI por strike/vencimiento,
   IV, griegas, trades con dirección — el plan ya especifica que esto va **después** de
   microestructura (fase 5), no antes, y que GEX se calcula "por escenario" con
   supuestos explícitos, nunca como hecho observado.
4. **Order flow del propio CLOB de Polymarket**: hoy nuestro
   [reconstructor L2](CLOB-L2-CONTRACT-V1.md) da profundidad de libro pero no procesa
   `last_trade_price` (evento ya presente en el RAW capturado, con `side`) para construir
   un order imbalance del mercado de predicción mismo, que es el equivalente más directo
   a lo que Anastasopoulos et al. miden como order flow.
5. Solo con (2)-(4) capturados, normalizados y con calidad validada tendría sentido
   diseñar el estudio de lead-lag/ablación que el plan exige (fase 4-5) antes de aceptar
   o rechazar que gamma/GEX aporta algo sobre el settlement de 5 minutos.

## Conclusión

- La intuición del trader tiene una base real si se refería a **delta/order flow de
  opciones** (Doan & Vo lo demuestra empíricamente para BTC/ETH a frecuencia horaria).
- Si se refería específicamente a **exposición gamma de dealers (GEX)**, la evidencia es
  más débil y mayormente no académica; nuestro propio plan ya trae esa cautela
  incorporada desde antes de esta sesión.
- No cambia el orden de fases del plan: sigue siendo registry → multi-venue →
  microestructura → opciones. Lo que sí cambia es que ahora hay una referencia
  concreta pendiente de leer (`sciencedirect.com/.../S1544612326008688`, sobre
  expiración de BTC y gamma) que debería revisarse antes de diseñar la fase 6.
