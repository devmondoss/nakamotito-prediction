# Anastasopoulos et al. (2026) — Order flow y retornos de criptomonedas

**Cita:** Alexia Anastasopoulos, Nikola Gradojevic, Fred Liu, Alex Maynard, Ilias
Tsiakas. "Order flow and cryptocurrency returns." *Journal of Financial Markets* 79
(2026) 101047. DOI: [10.1016/j.finmar.2026.101047](https://doi.org/10.1016/j.finmar.2026.101047).
Publicado online 2026-01-15. Acceso abierto (CC BY). Special issue "ML, AI, and Trading".

> Conversión a Markdown de un PDF provisto por el usuario. Resume metodología y
> resultados principales; no reproduce las tablas de robustez de doble-sort por moneda ni
> el Apéndice online. Ver el PDF original para el detalle completo y la bibliografía.

## Pregunta de investigación

¿El order flow (presión de compra/venta neta) explica y predice el cross-section de
retornos de criptomonedas? Los autores comparan tres hipótesis:

1. **No-predictability view**: el order flow no está relacionado con los retornos
   (hipótesis nula).
2. **Transitory view**: el order flow tiene efecto transitorio (liquidez, presión de
   precio, reversión de corto plazo).
3. **Permanent view**: el order flow tiene efecto permanente porque transmite
   información asimétrica al precio (Glosten-Milgrom 1985; Kyle 1985; Hasbrouck 1988,
   1991).

## Datos

- **84 criptomonedas** (cross-section balanceado), precios diarios en USD de
  coinmarketcap.com, 2018-01-01 a 2022-06-30. Criterio de inclusión: market cap > USD 1M
  al inicio, trading continuo con precio/volumen no-cero, se excluyen stablecoins.
- **Order flow** desde cryptocompare.com (CC): volumen firmado (buy/sell) —dato no
  disponible en CMC— denominado en **11 divisas fiat**: las G10 más Corea del Sur (por
  ser un centro relevante de trading cripto).
- `OF_i,t = of_i,t / σ(of_i,t−29:t)` — estandarizado por la volatilidad de order flow de
  los últimos 30 días (Menkhoff et al. 2016).
- **World order flow (`OF^W`)**: se agregan los 11 buy-volumes y los 11 sell-volumes, se
  toma la diferencia logarítmica y se estandariza igual que las series individuales.

### Descomposición de varianza de `OF^W` (Tabla 2)

| Divisa | % diario | % semanal |
|---|---|---|
| USD | 38.14 | 38.26 |
| KRW (Corea) | 23.21 | 30.48 |
| EUR | 10.57 | 10.09 |
| GBP | 5.95 | 4.98 |
| Resto (NZD, CHF, AUD, CAD, NOK, SEK, JPY) | ~16 | ~11 |

USD y KRW concentran el 61 % de la varianza; justifica incluir Corea aparte del G10.

## Metodología: panel regressions (in-sample)

### Contemporánea

`r_{i,t} = α + α₁·OF^W_{i,t} + controles + ε` (efectos fijos por coin, errores
clusterizados por tiempo y coin).

- Un aumento de 1 desviación estándar en `OF^W` se asocia a **+2.2 % de retorno diario**
  (t = 10.44) y **+4.0 % semanal** (t = 8.29). `R̄² = 10.4 %` diario, `19.6 %` semanal.

### Predictiva — separando componente transitorio vs. permanente

Siguiendo Bianchi et al. (2022): se usa el retorno rezagado como proxy del componente
transitorio (reversión de corto plazo), y se evalúa el order flow rezagado
**controlando por retorno rezagado**.

- `OF^W_{t-1}` solo, sin controlar por `r_{t-1}`: **no significativo** (t = 0.62) a
  frecuencia diaria.
- Controlando por `r_{t-1}`: **significativo** (t = 2.58–3.02). El retorno rezagado
  tiene coeficiente negativo y significativo (reversión).
- A frecuencia semanal, `OF^W_{t-1}` es significativo **con o sin** controlar por
  retorno rezagado (t = 2.30–3.40) — el componente permanente domina más a esa
  frecuencia.
- +1 desv. estándar en `OF^W_{t-1}` → **+0.2 % retorno diario futuro**, **+0.9 % semanal
  futuro**.

### World order flow vs. order flows individuales

En regresiones que condicionan en las 11 divisas + `OF^W` simultáneamente, `OF^W` tiene
siempre el coeficiente más alto y significativo; el order flow de EE. UU. deja de ser
significativo en presencia de `OF^W` en las regresiones predictivas — **world order
flow domina sobre cualquier order flow individual**, incluido el de EE. UU.

### Robustez: no-linealidades

Una única regresión con términos lineales, cuadráticos, cúbicos y de interacción
(`OF^W_{t-1}` × cada order flow) muestra: (a) varios términos no-lineales son
individualmente significativos, (b) son **conjuntamente significativos** (F-test
p = 0.000), y (c) el `R̄²` sube de 1.20 % (solo lineal) a 1.39 % (todos los términos) —
mayor sofisticación no-lineal aporta, sin diluir el efecto lineal de `OF^W`. Esto motiva
usar ML no-lineal para el forecasting out-of-sample.

### VAR (Hasbrouck 1991)

- El `OF^W` contemporáneo es altamente significativo para retornos en los 10 coins
  principales individualmente (BTC t = 12.05).
- El efecto predictivo es significativo para la mitad de los top-10 individualmente,
  pero fuerte en el panel agregado (top-10: t = 3.49; los 84: t = 5.39).
- Retorno rezagado → order flow futuro: relación negativa (reversión contraria /
  inventario), significativa en el agregado (t = −5.85 para 84 coins).

## Machine learning out-of-sample

### Modelos

Lineales: OLS, ridge (RR), lasso (LAS), elastic net (EN), principal component
regression (PCR). No-lineales: random forest (RF), stochastic gradient boosting (SGB),
redes neuronales de 1 a 4 capas (NN1-NN4). Combinaciones: L-Mean (promedio de lineales),
NL-Mean (promedio de no-lineales). Función de pérdida robusta de Huber (excepto RF, que
usa MSE por su robustez inherente a outliers).

### Esquema de validación

Ventana expansiva: entrenamiento 1 año → validación 1 año (hiperparámetros) → test 1
mes, avanzando 1 mes a la vez. Test total: 2020-02-18 a 2022-06-30. Solo forecasting
diario (la muestra es corta para ML semanal).

### Métrica

`R²_oos = 1 − Σ(r − r̂)² / Σ(r)²` contra el benchmark ingenuo de retorno cero (no
random-walk con drift). También `R¹_oos` basado en MAE (más robusto a outliers, dada la
altísima curtosis de los retornos cripto).

### Resultado principal (Tabla 10)

| Info set | Mejor modelo | `R²_oos` |
|---|---|---|
| Solo order flow (OF) | **SGB** | **0.39 %** |
| Solo fundamentales (EF) | SGB | 0.13 % |
| OF + EF | SGB | 0.14 % |

- **Todos** los modelos lineales que condicionan en OF tienen `R²_oos` negativo; **todos**
  los no-lineales lo tienen positivo. La complejidad del modelo importa.
- OF domina a EF en 11 de 13 especificaciones (`R²_oos`) y en las 13 (`R¹_oos`) — el
  order flow aporta más que los fundamentales económicos tradicionales (VIX, S&P 500,
  MSCI, tasas, spreads de crédito).

## Valor económico: portfolio sorts

- **Sort directo en `OF^W` semanal**: long-short (`P5-P1`) da **1.93 % semanal**
  (t = 2.99), alpha 1.78 % (t = 2.86) contra el modelo de 3 factores cripto (Liu et al.
  2022), Sharpe anualizado **2.05**.
- **Sort en `OF^W` diario, ortogonalizado contra el retorno rezagado** (para aislar el
  componente permanente): alpha 0.24 % diario (t = 1.65), Sharpe **1.00**. Sin
  ortogonalizar, el efecto diario es insignificante — confirma que hay que separar el
  componente transitorio para ver la señal permanente a esa frecuencia.
- **Sort en forecasts de ML (condicionando en las 11 divisas)**: el mejor, **SGB-OF**,
  da un long-short diario de 0.76 % (t = 5.54), alpha 0.78 % (t = 5.85), **Sharpe
  anualizado 3.68**. NL-Mean-OF: Sharpe 3.61.
- Superan a benchmarks de la literatura sin order flow: el NL-Mean de Filippou et al.
  (2024), que condiciona en 54 características sin order flow, da Sharpe 2.57; el mismo
  tipo de modelo condicionando solo en order flow da **3.61**.

### Portafolio long-only (sin shortear)

Dado que en cripto puede ser difícil o imposible shortear ciertos coins, el portafolio
**long-only** (`P5`) del modelo NL-Mean-OF da alpha diario 0.87 % (t = 3.18), Sharpe
1.98, con **break-even transaction cost de 1.07 % diario** — cómodamente por encima de
los costos reales estimados en cripto (0.3–0.5 %, Bianchi et al. 2022; Liu et al. 2022).

## Límites al arbitraje

Se construye un índice de límites al arbitraje (tamaño, volatilidad idiosincrática,
iliquidez de Amihud). Hallazgo clave:

- El `R²_oos` y el Sharpe de los modelos **no-lineales** condicionados en order flow son
  **robustos a través de todos los terciles** de dificultad de arbitraje (Sharpe 1.64,
  1.90, 2.93 de bajo a alto) — el efecto **no** se explica por límites al arbitraje.
- En cambio, el valor de los modelos **lineales** condicionados en order flow y de
  **cualquier** modelo condicionado en fundamentales económicos **sí** se concentra en
  coins difíciles de arbitrar (pequeños, ilíquidos, volátiles) — su desempeño **sí** se
  explica por límites al arbitraje.
- Restringiendo a los **10 coins principales por market cap** (los más fáciles de
  arbitrar), los resultados se sostienen igual de bien (SGB-OF Sharpe 2.14, NL-Mean-OF
  Sharpe 1.78) — refuerza que el efecto es genuino, no un artefacto de iliquidez.

## Conclusión del paper

El order flow —especialmente el `world order flow` agregado sobre 11 divisas— tiene
poder explicativo y predictivo fuerte y económicamente valioso para retornos cripto,
consistente con la **vista permanente** (información asimétrica), no con reversión
transitoria pura ni con ausencia de relación. Los modelos ML no-lineales que
condicionan en order flow superan tanto a los lineales como a benchmarks de la
literatura sin order flow, y el efecto sobrevive restricciones económicas realistas
(sin shorts, costos de transacción, límites al arbitraje).

## Relevancia para Nakamotito Prediction

- El concepto de **order flow** aquí (diferencia log de volumen buy/sell iniciado,
  estandarizada por volatilidad de 30 días) es directamente trasladable al CLOB de
  Polymarket una vez que capturemos trades con dirección — hoy nuestro
  [reconstructor L2](CLOB-L2-CONTRACT-V1.md) da profundidad de libro pero no procesa
  el evento `last_trade_price` (que sí viene en el RAW capturado, con `side`) para
  construir un order flow análogo.
- El patrón metodológico es replicable a nuestra escala: (1) separar componente
  transitorio de permanente ortogonalizando contra el retorno rezagado antes de
  evaluar valor económico; (2) usar modelos no-lineales (SGB en particular) recién
  cuando exista dataset suficiente — nuestro plan ya reserva esto para la fase 4
  (laboratorio base) en adelante; (3) walk-forward con ventanas de
  entrenamiento/validación/test expansivas, exactamente lo que el
  [plan cuantitativo](QUANT-SETTLEMENT-PLAN-2026-09-18.md) exige (purge/embargo, sección
  6).
- Advertencia de escala: esta evidencia es a frecuencia horaria/diaria sobre 84 coins
  durante 4.5 años. Nuestro contrato es una ronda de **5 minutos**; no se puede asumir
  que el mismo horizonte de "efecto permanente vs. transitorio" aplique sin validación
  propia — es justamente el tipo de generalización que el plan prohíbe hacer sin
  evidencia (ver límites en `QUANT-SETTLEMENT-PLAN-2026-09-18.md`, sección 10).
