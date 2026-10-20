import { describe, expect, it } from "vitest";
import type { EntryRecord, LiveState } from "@earendil-works/pi-durable";
import { toolActivity } from "./tool-activity.js";

describe("codemode activity IDs", () => {
  it("uses the same nested ID for queued progress and the final result", () => {
    const live: LiveState = { tools: [{ callId: "script", name: "codemode", status: "running", details: { calls: [{ id: 2, name: "get_account", status: "queued", args: "{}" }] } }] };
    const entries: EntryRecord[] = [{ id: 1 as EntryRecord["id"], kind: "pi.tool-result", conversationId: 1 as EntryRecord["conversationId"], model: [{ role: "toolResult", toolCallId: "script", toolName: "codemode", content: [], isError: false, timestamp: 1, details: { calls: [{ id: 2, name: "get_account", status: "ok", args: "{}" }] } }] }];
    expect(toolActivity([], live)).toMatchObject([{ id: "script", status: "running" }, { id: "script/c2", status: "queued" }]);
    expect(toolActivity(entries)).toMatchObject([{ id: "script", status: "done" }, { id: "script/c2", status: "ok" }]);
  });
});
