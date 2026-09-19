# Doan & Vo (2025) — Net buying pressure entre derivados y spot

**Cita:** Bao Doan, Duc Hong Vo. "The interconnection between net buying pressures in
derivatives and spot markets." *The British Accounting Review*, xxx (xxxx) xxx.
DOI: [10.1016/j.bar.2025.101702](https://doi.org/10.1016/j.bar.2025.101702). Recibido
2023-11-02, aceptado 2025-07-04, publicado online 2025-07-09. Acceso abierto (CC BY 4.0).

> Conversión a Markdown de un PDF provisto por el usuario, para referencia dentro del
> repo. Resume metodología y resultados; **no reproduce las ~13 tablas de robustez del
> Apéndice número por número** (serían miles de coeficientes sin valor incremental para
> este proyecto) — se referencian donde son relevantes. Ver el PDF original para el
> detalle completo y la bibliografía.
>
> **Nakamotito Prediction es un proyecto exclusivamente BTC.** El paper original
> estudia BTC y ETH en paralelo; esta nota conserva solo los resultados de **BTC** y
> menciona ETH nada más cuando hace falta para entender el contraste metodológico (p.
> ej. cuándo el paper encuentra que un efecto es específico de BTC).

## Pregunta de investigación

¿El order imbalance (presión de compra neta) en un mercado —spot, futuros u
opciones— mejora la predictibilidad del order imbalance en los otros mercados? Y si
existe ese efecto cruzado, ¿qué lo explica: el canal de **liquidez** o el de
**sentimiento de mercado**?

## Datos

- **Universo del paper:** BTC y ETH, frecuencia horaria, enero 2019 – diciembre 2022
  (esta nota solo conserva BTC).
- **Spot y futuros perpetuos:** Binance (BTCUSDT) — timestamp, precio, volumen,
  indicador buy/sell.
- **Opciones:** Deribit (vía Laevitas) — timestamp, precio, volumen, buy/sell, IV, strike,
  vencimiento, precio subyacente al momento del trade.
- Se filtran opciones con IV entre 0 % y 500 %, precio y volumen positivos, y delta
  absoluto entre 0.02 y 0.98 (fuera de ese rango: precios rancios / iliquidez).

## Definición de variables

### Order imbalance de spot y futuros

Diferencia entre volumen iniciado por comprador y por vendedor en el intervalo
`(t-1, t]`, en miles: `SImb_t`, `FImb_t`.

### Delta de Black-Scholes (para clasificar moneyness)

```
ΔC_t(S_t, σ | K, T) = Φ[ ln(S_t·e^{rτ}/K) + 0.5·σ²·τ  /  (σ·√τ) ]
ΔP_t(S_t, σ | K, T) = ΔC_t(S_t, σ | K, T) − 1
```

- `σ`: raíz cuadrada anualizada de la varianza realizada diaria promedio de los últimos
  15 días (suma de retornos horarios al cuadrado por día).
- `τ = T − t`: tiempo a vencimiento en años.
- `r`: tasa libre de riesgo interpolada linealmente entre tasas FRED de 1 día, 1, 3, 6 y
  12 meses.

### Categorías de moneyness (por `|Δ|`)

| Categoría | Rango de `|Δ|` |
|---|---|
| Deep-OTM (DOTM) | 0.02 – 0.125 |
| OTM | 0.125 – 0.375 |
| ATM | 0.375 – 0.625 |
| ITM | 0.625 – 0.875 |
| Deep-ITM (DITM) | 0.875 – 0.98 |

### Order imbalance de opciones

```
OImb_t = Σ Buy_{j,t}·|Δ_{j,t}|  −  Σ Sell_{j,t}·|Δ_{j,t}|
```

### Demanda direccional vs. de volatilidad (Chen & Wang 2017; Alexander et al. 2023)

Para cada tipo de opción `i ∈ {call, put}` y moneyness `k ∈ {ATM, OTM, DOTM}`:

```
A^k_{i,t} = Σ Buy^k_{i,j,t}·|Δ^k_{i,j,t}|  −  Σ Sell^k_{i,j,t}·|Δ^k_{i,j,t}|

demanda direccional  D[k]_{C,t} = (A^k_{C,t} − A^k_{P,t}) / 2
demanda de volatilidad V[k]_{C,t} = (A^k_{C,t} + A^k_{P,t}) / 2
```

Una demanda direccional positiva en calls anticipa una **subida del precio**; una
demanda de volatilidad positiva anticipa una **subida de la IV**. No se mide para ITM/DITM
por su bajo volumen (~6.35 % del total).

## Estadísticas descriptivas (resumen de la Tabla 1)

| Variable | BTC (media) |
|---|---|
| Retorno horario | 0.0069 % |
| Δ IV horaria | −0.0082 % |
| Δ RV horaria | −0.0012 % |
| SImb (spot) | −0.0299 |
| FImb (futuros) | −0.0361 |
| OImb (opciones) | 0.0041 |
| D[ATM] | 0.0002 |
| V[ATM] | 0.0008 |

Spot y futuros tienen imbalance promedio **negativo** (consistente con S&P 500 en la
literatura); las opciones tienen imbalance promedio **positivo** (lo opuesto al mercado
de acciones de EE. UU.). La causalidad de Granger es significativa (p < 5 %) para
**los tres pares** de mercados de BTC (spot↔futuros, spot↔opciones, futuros↔opciones).

## Resultados principales

### 1. Contenido informativo de cada mercado por separado (regresión ec. 1)

`y_{t+i} = α + α₁·y_t + α₂·x_t + ε`, con `x_t ∈ {SImb, FImb, OImb}`, `i ∈ {1,2,5}`.

- Un OImb positivo **no predice de forma robusta el retorno futuro de BTC** (el paper sí
  encuentra ese efecto en ETH, que no es relevante aquí).
- OImb predice **mayor volatilidad futura de BTC** (implícita y realizada), significativo
  al 1 %.
- El efecto es fuerte a un período adelante y se diluye a 2 y 5 períodos — la
  información de order imbalance es de corto plazo.

### 2. Impacto cruzado: spot/futuros → predictibilidad de opciones (ec. 3-4)

Con dummies de extremos (top/bottom 30-percentil) de SImb/FImb interactuadas con OImb:

- En BTC, un OImb alto durante un movimiento extremo de SImb (o FImb) predice **retornos
  futuros más altos (bajos)** — el efecto es más fuerte con opciones ATM.
- Para volatilidad: un OImb alto en momentos de cambio extremo del order imbalance
  spot/futuros predice **mayor volatilidad futura** en BTC, con coeficientes hasta el
  doble del efecto standalone.

### 3. Impacto cruzado: demanda direccional/volatilidad de opciones → spot/futuros (ec. 5-6)

- La demanda direccional en OTM predice retornos futuros de BTC positivamente.
- La demanda de volatilidad predice mayor volatilidad futura de BTC, consistentemente a
  través de moneyness.
- El order imbalance de spot/futuros pierde poder predictivo standalone sobre
  volatilidad cuando se controla por estas demandas — sugiere que la info de opciones
  domina en ese margen.

### 4. VAR — causalidad bidireccional (Hasbrouck 1991)

- El order imbalance de opciones **contemporáneo** es altamente significativo para el
  retorno de BTC (t-stat = 12.05).
- El efecto **predictivo** (lag 1) de BTC individualmente no es significativo por sí
  solo, pero el paper también corre el VAR sobre un panel agregado de 84 coins donde sí
  lo es (t = 5.39) — el efecto de BTC solo es más débil que el del agregado.
- El retorno rezagado de BTC predice **negativamente** el order imbalance futuro
  (reversión contraria / efecto de inventario), t = −2.83.

### 5. Canales explicativos

- **Liquidez** (Corwin-Schultz 2012, Abdi-Ranaldo 2017): en BTC, los traders informados
  prefieren operar en opciones cuando el mercado spot subyacente está **líquido**
  (patrón intradiario "alta liquidez → precio más informativo", Admati & Pfleiderer
  1988).
- **Sentimiento** (Google Trends "Bitcoin", Fear & Greed Index): con sentimiento muy
  positivo y cambios extremos en el order imbalance spot, un OImb más alto predice
  **retornos más altos**. Con sentimiento muy negativo, el order imbalance de opciones es
  más informativo para interpretar el poder predictivo del spot sobre retorno y
  volatilidad.

## Conclusiones del paper

1. El order imbalance de opciones aporta información incremental sobre spot/futuros y
   viceversa (a diferencia del mercado de acciones, donde la literatura encuentra que
   las opciones no aportan información incremental al spot).
2. Para BTC específicamente, el efecto es más consistente sobre **volatilidad futura**
   que sobre dirección del retorno.
3. La adquisición de Deribit por Coinbase (2025) podría acentuar el rol de precio de
   opciones en el descubrimiento de precio institucional.
4. Limitación reconocida por los propios autores: con machine learning (elastic net,
   árboles, random forest, boosting, redes neuronales) el desempeño **no** mejora sobre
   OLS en su muestra — atribuido a la alta frecuencia/corta ventana temporal (4 años
   horarios) y al número limitado de predictores (a diferencia de Gu et al. 2020, que
   usa +900 predictores a frecuencia mensual). Dejan el ML para trabajo futuro con
   muestra más larga y más predictores.

## Relevancia para Nakamotito Prediction

- **Las "griegas" que usa este paper son el delta de Black-Scholes**, no la gamma. Se
  usa exclusivamente para (a) clasificar moneyness y (b) ponderar el volumen
  buy/sell al construir `OImb`, `D[k]`, `V[k]`. **No hay ningún cálculo de exposición
  gamma (GEX) ni de dealer hedging en este paper.** Si el trader que mencionó "griegas y
  gammas" se refería a usar el delta para leer el order flow de opciones, esto es
  evidencia real y peer-reviewed de que funciona parcialmente (para volatilidad futura,
  de forma más consistente que para dirección). Si se refería a exposición gamma de
  dealers (GEX), este paper no lo estudia — ver la nota de síntesis
  [RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md](RESEARCH-NOTES-2026-09-19-ORDER-FLOW-AND-GREEKS.md).
- El diseño (order imbalance por moneyness, demanda direccional vs. de volatilidad,
  descomposición por canal de liquidez/sentimiento) es exactamente el tipo de feature
  que la fase 6 (opciones) del [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md)
  contempla — y confirma la distinción que el plan ya hace entre "régimen de
  volatilidad" y "afirmación de posición de dealer observada".
- Requeriría, antes de poder replicar algo similar sobre nuestro contrato BTC Up/Down
  5m: (1) un collector de Deribit con trades etiquetados buy/sell + IV + delta (fase 6,
  no iniciada); (2) collectors de spot/perp de Binance/Bybit con order imbalance propio
  (fase 3, no iniciada). Nuestro reconstructor L2 actual (`src/l2.rs`) da profundidad de
  libro del CLOB de predicción pero no calcula order imbalance de trades todavía.
