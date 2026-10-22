import { Type } from "@earendil-works/pi-ai";
import { defineExtension, defineTool, section } from "@earendil-works/pi-durable";
import type { ExecutionContextService } from "../agents/execution-context.js";
import { jsonResult } from "../research/tool-response.js";
import type { NotificationService } from "./notification-service.js";

export function createNotificationExtension(agentId: string, notifications: NotificationService, execution: ExecutionContextService) {
  return defineExtension({
    name: "notifications",
    tools: [defineTool({
      name: "send_notification",
      description: "Send a concise user-facing notification to this Agent's web history and linked Telegram chat. Returns queued (not proof of Telegram delivery) or disabled. Backend order submission/fill notifications are automatic when enabled; do not duplicate them. Do not include credentials or secrets. Cite full review/report references in the body where useful. This is one-way, not a question/approval tool.",
      parameters: Type.Object({
        title: Type.String({ minLength: 1, maxLength: 200 }), body: Type.String({ minLength: 1, maxLength: 12000 }),
        severity: Type.Optional(Type.Union([Type.Literal("info"), Type.Literal("warning"), Type.Literal("error")])),
      }, { additionalProperties: false }),
      structuredOutputSchema: Type.Union([Type.Object({ status: Type.Literal("queued"), id: Type.String() }), Type.Object({ status: Type.Literal("disabled") })]),
      replay: "safe", executionMode: "sequential",
      execute: async (args, api, context) => jsonResult(await notifications.sendFromTool((await execution.resolve(agentId, api, context)).actor, args, execution)),
    })],
    sections: [section("notifications", () => "send_notification is a one-way message to the user, not permission to trade or an interactive question. Queued means stored, not delivered or read. If disabled, continue the assignment without retrying or working around the setting. Do not duplicate automatic order submission/fill notifications. After a review is published and intended insight edits finish, send one crafted summary with the review reference, exact window/coverage, findings, actual changes versus recommendations, and unresolved limitations. Notification failures must not cause an exchange action to be repeated.")],
  });
}
