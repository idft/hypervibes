import { describe, expect, it } from "vitest";
import { describeDataset, researchReportExample, technicalEvidenceInstructions } from "./evidence.js";

describe("actual dataset provenance", () => {
  it("uses the actual rows, not an inferred request range or expected count", () => {
    expect(describeDataset([{ time: 3600 }, { time: 10800 }])).toEqual({ count: 2, firstTime: 3600, lastTime: 10800 });
  });
  it("does not fabricate timestamps for empty datasets", () => {
    expect(describeDataset([])).toEqual({ count: 0, firstTime: null, lastTime: null });
  });
  it("provides the tested report example without imposing a report schema or monetary coercion", () => {
    expect(technicalEvidenceInstructions).toContain(researchReportExample);
    expect(technicalEvidenceInstructions).toContain("optional code, not a required report schema");
    expect(technicalEvidenceInstructions).toContain("keep ISO time and memory references outside the indicators object");
    expect(technicalEvidenceInstructions).toContain("limitation-only report without fabricated numbers");
    expect(technicalEvidenceInstructions).toContain("keep monetary values as exact decimal strings");
  });
});
