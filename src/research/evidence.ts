/** Actual candle-open bounds in UTC seconds, not requested millisecond boundaries. */
export function describeDataset(rows: readonly { time: number }[]) {
  return { count: rows.length, firstTime: rows[0]?.time ?? null, lastTime: rows.at(-1)?.time ?? null };
}
export const utcFormattingExample = `function utcMilliseconds(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toISOString() : null;
}
function utcSeconds(value) { return typeof value === 'number' ? utcMilliseconds(value * 1000) : null; }
function datasetEvidence(dataset) {
  return dataset ? {...dataset, firstTimeUtc:utcSeconds(dataset.firstTime ?? dataset.firstOpenTime), lastTimeUtc:utcSeconds(dataset.lastTime ?? dataset.lastOpenTime)} : null;
}`;
export const technicalEvidenceExample = `${utcFormattingExample}
const latest = candleResult.candles.at(-1);
const priceValid = candleResult.coverage.complete && candleResult.nextStart === null && latest && Number.isFinite(latest.close);
function scalar(result, indicator, period, output) {
  const point = result.values?.at(-1), value = point?.value?.[output];
  return priceValid && result.coin === candleResult.coin && result.interval === candleResult.interval &&
    result.indicatorId === 'builtin:' + indicator && result.resolvedInputs.period === period &&
    (!('source' in result.resolvedInputs) || result.resolvedInputs.source === 'close') &&
    result.coverage.complete && point?.time === latest.time && typeof value === 'number' && Number.isFinite(value) ? value : null;
}
const ema20 = scalar(ema20Result, 'ema', 20, 'EMA'), ema50 = scalar(ema50Result, 'ema', 50, 'EMA');
const atr = scalar(atrResult, 'atr', atrResult.resolvedInputs?.period, 'ATR');
function provenance(result) {
  return {coin:result.coin, interval:result.interval, requestedStartUtc:utcMilliseconds(result.start), effectiveEndExclusiveUtc:utcMilliseconds(result.end),
    input:datasetEvidence(result.dataset), output:datasetEvidence(result.outputDataset), coverage:result.coverage,
    inputs:result.resolvedInputs ?? null, version:result.version ?? null, sourceHash:result.sourceHash ?? null, resultHash:result.resultHash ?? null};
}
text({datasets:{candles:provenance(candleResult), ema20:provenance(ema20Result), ema50:provenance(ema50Result), atr:provenance(atrResult)},
  latestClosed:latest ? {...latest, openTimeUtc:utcSeconds(latest.time)} : null,
  facts:{priceBelowEma20:ema20 === null ? null : latest.close < ema20,
    ema20AboveEma50:ema20 === null || ema50 === null ? null : ema20 > ema50,
    atrPercent:atr === null || latest.close <= 0 ? null : atr / latest.close * 100}});`;
export const bollingerEvidenceExample = `const bandPoint = bollingerResult.values?.at(-1), bands = bandPoint?.value;
const bandsValid = priceValid && bollingerResult.coin === candleResult.coin && bollingerResult.interval === candleResult.interval &&
  bollingerResult.indicatorId === 'builtin:bollinger' && bollingerResult.resolvedInputs.source === 'close' &&
  bollingerResult.coverage.complete && bandPoint?.time === latest.time && bands &&
  ['Lower','Middle','Upper'].every(key => typeof bands[key] === 'number' && Number.isFinite(bands[key])) && bands.Lower <= bands.Middle && bands.Middle <= bands.Upper;
text({bollinger:{openTimeUtc:utcSeconds(bandPoint?.time), value:bandsValid ? bands : null, provenance:provenance(bollingerResult)},
  facts:{priceAboveUpperBand:bandsValid ? latest.close > bands.Upper : null, priceBelowLowerBand:bandsValid ? latest.close < bands.Lower : null}});`;
export const technicalEvidenceInstructions = `Calculate comparison flags in Code Mode before writing prose; Check the final prose against the flags. Facts are not forecasts. Quote the emitted ISO UTC strings verbatim. Request start/end are milliseconds, candle-open times are seconds. All indicators use point.value.<outputId>: EMA, ATR, RSI, ADX, MACD/Signal/Histogram and Lower/Middle/Upper. Null/missing/nonfinite/misaligned/non-close-source evidence is unknown, never zero or fabricated false. Report actual seed/output bounds, coverage, resolvedInputs, concrete version/sourceHash/resultHash and per-output finite availability. A finite value does not prove initialization effects have disappeared. About 5× the longest period plus trailing history is a seed recommendation, not universal stability. A one-value request still computes the entire seed window. No universal stable/requiredInputs claim exists for arbitrary PineScript. Use indicator_get for exact keys. Example (bind these results using selected tools on a common closed window):\n\n${technicalEvidenceExample}\n\n${bollingerEvidenceExample}`;
