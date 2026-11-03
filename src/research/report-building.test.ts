import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { describe, expect, it, vi } from "vitest";
import { researchReportExample, utcFormattingExample } from "./evidence.js";

const bindings = `${utcFormattingExample}
const priceValid = true, latest = {close: 123.456};
const candleResult = {end: 1735693200000};
const ema20 = 120, ema50 = 110, atr = 2;
`;
const endUtc = "2025-01-01T01:00:00.000Z";

async function execute(tail: string, setup = bindings) {
  const publish = vi.fn(() => ({ ref: "test-only" }));
  const sandbox = new CodemodeSandbox({ tools: [{ name: "publish_research", execute: publish }] });
  try {
    const result = await sandbox.execute(`${setup}\n${researchReportExample}\n${tail}`);
    return { result, publish };
  } finally {
    await sandbox.close();
  }
}

describe("runnable research report example", () => {
  it("selects the numeric close, renders interpolation, and emits a draft with separate ISO time", async () => {
    const { result, publish } = await execute(`
      await tools.publish_research(researchDraft);
      return {draft: researchDraft, samePayload: researchDraft.payload === reportData};
    `);
    const draft = {
      content: `Close: 123.456; candle end UTC: ${endUtc}. ema20: 120; ema50: 110; atr: 2.`,
      payload: {
        closePrice: 123.456, candleEndUtc: endUtc,
        indicators: { ema20: 120, ema50: 110, atr: 2 }, previousResearchRef: null,
      },
    };
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value).toEqual({ draft, samePayload: true });
    expect(result.output).toHaveLength(1);
    const output = result.output[0]!;
    expect(output.type).toBe("text");
    if (output.type === "text") expect(JSON.parse(output.text)).toEqual(draft);
    expect(publish).toHaveBeenCalledExactlyOnceWith(draft, expect.anything());
  });

  it("keeps an unavailable close unknown, preserves zero, and does not round tiny values to zero", async () => {
    const { result } = await execute("return researchDraft;", `${utcFormattingExample}
      const priceValid = false, latest = {close: 999};
      const candleResult = {end: 1735693200000};
      const ema20 = null, ema50 = 0, atr = 1e-12;
    `);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value).toEqual({
      content: `Close: unknown; candle end UTC: ${endUtc}. ema20: unknown; ema50: 0; atr: 1e-12.`,
      payload: {
        closePrice: null, candleEndUtc: endUtc,
        indicators: { ema20: null, ema50: 0, atr: 1e-12 }, previousResearchRef: null,
      },
    });
  });

  it("renders only indicators and permits sibling references and legitimate prose with literal templates or NaN", async () => {
    const note = "NaN denotes unavailable evidence; a literal template example is ${value}.";
    const { result, publish } = await execute(`
      const data = {...reportData, previousResearchRef: 'm17', note: ${JSON.stringify(note)}};
      const draft = buildResearchReport(data);
      draft.content += '\\n' + data.note;
      await tools.publish_research(draft);
      return {draft, samePayload: draft.payload === data};
    `);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value).toEqual({
      samePayload: true,
      draft: {
        content: `Close: 123.456; candle end UTC: ${endUtc}. ema20: 120; ema50: 110; atr: 2.\n${note}`,
        payload: {
          closePrice: 123.456, candleEndUtc: endUtc,
          indicators: { ema20: 120, ema50: 110, atr: 2 }, previousResearchRef: "m17", note,
        },
      },
    });
    expect(publish).toHaveBeenCalledOnce();
  });

  it.each([
    "'2025-01-01T01:00:00.000Z'", "'123.456'", "NaN", "Infinity", "-Infinity",
    "undefined", "[]", "[123]", "{}", "{previousResearchRef: 'm17'}", "true", "false",
  ])("rejects analytical value %s before publication", async (value) => {
    for (const expression of [
      `analyticalText(${value}, 'ema20')`,
      `buildResearchReport({...reportData, closePrice: ${value}})`,
      `buildResearchReport({...reportData, indicators: {ema20: ${value}}})`,
    ]) {
      const { result, publish } = await execute(`const checked = ${expression}; await tools.publish_research(checked);`);
      expect(result.ok, expression).toBe(false);
      if (!result.ok) {
        expect(result.error.kind).toBe("script");
        expect(result.error.message).toContain("must be a finite analytical number or null");
      }
      expect(publish).not.toHaveBeenCalled();
      expect(result.calls).toEqual([]);
    }
  });

  it("rejects metadata accidentally placed inside indicators before publication", async () => {
    const { result, publish } = await execute(`
      const data = {...reportData, indicators: {...reportData.indicators, previousResearchRef: 'm123'}};
      await tools.publish_research(buildResearchReport(data));
    `);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.kind).toBe("script");
      expect(result.error.message).toContain("previousResearchRef must be a finite analytical number or null");
    }
    expect(publish).not.toHaveBeenCalled();
    expect(result.calls).toEqual([]);
  });

  it.each([
    "undefined", "null", "[]", "1", "'report'", "true", "{}",
    "{...reportData, indicators: undefined}", "{...reportData, indicators: null}",
    "{...reportData, indicators: []}", "{...reportData, indicators: 'ema20'}",
    "{...reportData, indicators: 1}", "{...reportData, indicators: true}",
  ])("rejects missing or invalid report/indicator containers: %s", async (data) => {
    const { result, publish } = await execute(`await tools.publish_research(buildResearchReport(${data}));`);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.kind).toBe("script");
      expect(result.error.message).toContain("Expected report data with a separate indicators object");
    }
    expect(publish).not.toHaveBeenCalled();
    expect(result.calls).toEqual([]);
  });
});
