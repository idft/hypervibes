import type { ChartDrawing } from "./contracts.js";
export interface ScreenPoint { x: number; y: number }
export function lineEnds(kind: ChartDrawing["kind"], a: ScreenPoint, b: ScreenPoint, width: number): [ScreenPoint, ScreenPoint] {
  if (kind === "horizontal") return [{ x: 0, y: a.y }, { x: width, y: a.y }];
  if (kind === "horizontal-ray") return [a, { x: width, y: a.y }];
  const dx = b.x - a.x;
  if (!dx || (kind !== "ray" && kind !== "extended")) return [a, b];
  const at = (x: number) => ({ x, y: a.y + (b.y - a.y) * (x - a.x) / dx });
  if (kind === "extended") return [at(0), at(width)];
  return [a, at(dx > 0 ? width : 0)];
}
