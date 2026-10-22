export interface NotificationSettings { enabled: boolean; submissions: boolean; fills: boolean }
export type NotificationSeverity = "info" | "warning" | "error";
export type NotificationCategory = "message" | "submission" | "fill";
export type NotificationDelivery = "web_only" | "pending" | "sent" | "failed" | "suppressed";
export interface NotificationInput { title: string; body: string; severity?: NotificationSeverity }
export interface NotificationRecord {
  id: string; agentId: string; category: NotificationCategory; title: string; body: string; severity: NotificationSeverity;
  source: string; subagentId: string | null; runId: string | null; conversationId: number | null; orderRequestId: string | null;
  createdAt: number; delivery: NotificationDelivery; attempts: number; sentAt: number | null; error: string | null;
}
export interface NotificationPage { items: NotificationRecord[]; page: number; pages: number; total: number }
export type NotificationResult = { status: "queued"; id: string } | { status: "disabled" };
