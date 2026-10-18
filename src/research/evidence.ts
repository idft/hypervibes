/** Actual candle-open bounds in UTC seconds, not requested millisecond boundaries. */
export function describeDataset(rows: readonly { time: number }[]) {
  return { count: rows.length, firstTime: rows[0]?.time ?? null, lastTime: rows.at(-1)?.time ?? null };
}

/** Copyable QuickJS helpers: seconds and milliseconds must never be interchanged. */
export const utcFormattingExample = `function utcMilliseconds(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toISOString() : null;
}
function utcSeconds(value) {
  return typeof value === 'number' ? utcMilliseconds(value * 1000) : null;
}
function datasetEvidence(dataset) {
  return dataset ? {...dataset, firstTimeUtc:utcSeconds(dataset.firstTime), lastTimeUtc:utcSeconds(dataset.lastTime)} : null;
}`;

/** Copyable QuickJS example, tested against real tool results; not executed on the host. */
export const technicalEvidenceExample = `${utcFormattingExample}
const latest = candleResult.candles.at(-1);
const priceValid = candleResult.coverage.complete && candleResult.nextStart === null && latest && Number.isFinite(latest.close);
function scalar(result, indicator, period) {
  const point = result.values?.at(-1);
  return priceValid && result.coin === candleResult.coin && result.interval === candleResult.interval &&
    result.indicator === indicator && result.parameters.period === period &&
    (!('source' in result.parameters) || result.parameters.source === 'close') &&
    result.coverage.complete && result.stable && point?.time === latest.time &&
    typeof point.value === 'number' && Number.isFinite(point.value) ? point.value : null;
}
const ema20 = scalar(ema20Result, 'ema', 20), ema50 = scalar(ema50Result, 'ema', 50);
const atr = scalar(atrResult, 'atr', atrResult.parameters?.period);
function provenance(result) {
  return {coin:result.coin, interval:result.interval, requestedStartMs:result.start, effectiveEndMs:result.end,
    requestedStartUtc:utcMilliseconds(result.start), effectiveEndExclusiveUtc:utcMilliseconds(result.end),
    input:datasetEvidence(result.dataset), output:datasetEvidence(result.outputDataset), coverage:result.coverage,
    parameters:result.parameters ?? null, stable:result.stable ?? null, requiredInputs:result.requiredInputs ?? null};
}
text({datasets:{candles:provenance(candleResult), ema20:provenance(ema20Result), ema50:provenance(ema50Result), atr:provenance(atrResult)},
  latestClosed:latest ? {...latest, openTimeUtc:utcSeconds(latest.time)} : null,
  facts:{priceBelowEma20:ema20 === null ? null : latest.close < ema20,
    ema20AboveEma50:ema20 === null || ema50 === null ? null : ema20 > ema50,
    atrPercent:atr === null || latest.close <= 0 ? null : atr / latest.close * 100}});`;

/** Optional continuation when Bollinger was selected; reuse the helpers/results above. */
export const bollingerEvidenceExample = `const bandPoint = bollingerResult.values?.at(-1);
const bands = bandPoint?.value; // Not bandPoint.lower: every point is {time, value}.
const bandsValid = priceValid && bollingerResult.coin === candleResult.coin && bollingerResult.interval === candleResult.interval &&
  bollingerResult.indicator === 'bollinger' && bollingerResult.parameters.source === 'close' &&
  bollingerResult.coverage.complete && bollingerResult.stable && bandPoint?.time === latest.time && bands &&
  ['lower','middle','upper'].every(key => typeof bands[key] === 'number' && Number.isFinite(bands[key])) &&
  bands.lower <= bands.middle && bands.middle <= bands.upper;
text({bollinger:{openTimeUtc:utcSeconds(bandPoint?.time), value:bandsValid ? {lower:bands.lower,middle:bands.middle,upper:bands.upper} : null,
  provenance:provenance(bollingerResult)},
  facts:{priceAboveUpperBand:bandsValid ? latest.close > bands.upper : null,
    priceBelowLowerBand:bandsValid ? latest.close < bands.lower : null}});`;

export const technicalEvidenceInstructions = `Before writing analysis, calculate explicit comparison flags and percentages in Code Mode; do not infer numeric relationships while composing prose. Keep facts distinct from interpretive support/resistance or forecasts. Print separate provenance for candle inspection and EACH indicator: dataset.count/firstTime/lastTime describe actual input candles (UTC seconds), outputDataset describes returned values, start/end are requested-start/effective-exclusive-end milliseconds. Before emitting evidence, convert candle/dataset seconds with new Date(time * 1000).toISOString(), and start/end milliseconds with new Date(time).toISOString(). Quote the emitted ISO UTC strings verbatim in prose; never mentally convert epochs, apply local timezone offsets or invent a through-HH:59 timestamp. Label candle OPEN time separately from the exclusive range END; only a complete, aligned last candle ends at that range end. All indicator values[] entries are {time, value}: scalar indicators use point.value; Bollinger uses point.value.lower/middle/upper and MACD point.value.macd/signal/histogram, never point.lower or point.macd. Missing fields are unknown, not zero. Include parameters, coverage, requiredInputs and stable. Never describe a 119-bar inspection window as a 179-bar indicator seed window. Use a shared fixed range where practical; disclose different seed ranges. For the close-price comparison flags illustrated here, missing, unstable, non-finite, non-close-source or misaligned evidence must produce null/unknown, never a fabricated false flag. Check the final prose against the flags and numbers; a true ema20AboveEma50 cannot be described as EMA20 below EMA50. This example assumes candleResult, ema20Result, ema50Result and atrResult are results from selected tools; adapt periods/indicators to the assignment, not a mandatory strategy:\n\n\`\`\`js\n${technicalEvidenceExample}\n\`\`\`\nIf Bollinger is selected, bind bollingerResult to its result and optionally continue with this example; do not refetch an already available result merely to inspect its shape:\n\n\`\`\`js\n${bollingerEvidenceExample}\n\`\`\``;
