import type { ReactNode } from "react";

/** The screenshot build substitutes this module before the live stream code is imported. */
export function AgentEventsProvider({ children }: { children: ReactNode }) { return children; }
export function useAgentEvent() { /* Static fixture: no stream and no updates. */ }
const status = { interrupted: false, controlMessage: "", streamErrors: {}, reconnect() { /* No connection exists. */ } };
export function useAgentEventsStatus() { return status; }
