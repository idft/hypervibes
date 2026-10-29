import { defineExtension, section } from "@earendil-works/pi-durable";

// These examples are also executed in the real Code Mode sandbox by tests.
export const timeCodeExamples = `const scriptTimeMs = Date.now(); // Replay-safe script clock, not a continuously ticking wall clock.
const windowForHours = (endMs, hours) => ({ start: endMs - hours * 3_600_000, end: endMs });
const previousUtcDay = (anchorMs) => {
  const end = Math.floor(anchorMs / 86_400_000) * 86_400_000;
  return { start: end - 86_400_000, end };
};
const toUtcIso = (timestampMs) => new Date(timestampMs).toISOString();
const fromUtcIso = (isoWithZ) => Date.parse(isoWithZ);`;

export const timeGuidance = `Use UTC for all dates, calendar boundaries and summaries. Interpret dates without a timezone as UTC. Yesterday means the previous UTC calendar day, not the preceding 24 hours. Use ISO 8601 strings ending in Z for parsing and label human-readable times UTC. Do not use local-time Date constructors or local getters/setters.
Tool timestamps use Unix epoch milliseconds unless a field explicitly documents another unit. Existing candle, indicator point and chart-anchor time fields use epoch seconds: convert those explicitly (seconds * 1000, milliseconds / 1000). Durations are not timestamps; retain explicitly named units such as timeoutSeconds.
1 second = 1,000 ms; 1 minute = 60 seconds = 60,000 ms; 1 hour = 3,600 seconds = 3,600,000 ms; 1 day = 86,400 seconds = 86,400,000 ms.
Never guess Unix timestamps or do date arithmetic mentally. When Code Mode is available, compute and convert in code. Date.now() is available: it starts at the recorded script start time, can advance with tool results, and is replay-safe, not a continuously ticking wall clock. Use it for a current-time window only when no explicit or assignment anchor applies; store the window once. Prefer explicit requested dates or a supplied scheduled endMs; otherwise use host admittedAtMs for an assignment, or currentTimeMs for a current chat question. Historical timestamps and examples are not the current clock. Freeze start/end once and preserve them across pages and retries. Follow each tool's documented boundary semantics. Validate parsed dates with Number.isFinite before calling tools. Pass computed variables directly to tools; use store/load to retain windows across scripts, rather than printing and manually retyping timestamps.
Code Mode examples (helper functions; supply the actual requested duration and authoritative anchor):
\`\`\`js
${timeCodeExamples}
\`\`\`
Use windowForHours(anchorMs, requestedHours) for a rolling window or previousUtcDay(anchorMs) for yesterday. Store the result, then pass its start/end directly to selected tools. These helpers do not call tools or grant additional permissions.`;

export function currentTimeContext(nowMs: number): string {
  return `Host currentTimeMs: ${nowMs} (${new Date(nowMs).toISOString()}). Host clock at request preparation, not a historical evidence boundary. Only the latest current_time section is the current clock.`;
}

export const timeGuidanceExtension = defineExtension({
  name: "time-guidance",
  sections: [
    section("time_handling", () => timeGuidance),
    section("current_time", () => currentTimeContext(Date.now())),
  ],
});
