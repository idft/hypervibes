import { describe, expect, it } from "vitest";
import { describeDataset } from "./evidence.js";

describe("actual dataset provenance", () => {
  it("uses the actual rows, not an inferred request range or expected count", () => {
    expect(describeDataset([{ time: 3600 }, { time: 10800 }])).toEqual({ count: 2, firstTime: 3600, lastTime: 10800 });
  });
  it("does not fabricate timestamps for empty datasets", () => {
    expect(describeDataset([])).toEqual({ count: 0, firstTime: null, lastTime: null });
  });
});
