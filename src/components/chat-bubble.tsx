import type { ReactNode } from "react";

export function ChatBubble({ role, children }: { role: "user" | "assistant" | "system" | "tool"; children: ReactNode }) {
  return <article className={`space-y-3 rounded-xl p-4 ${role === "user" ? "ml-8 bg-violet-950/40" : role === "assistant" ? "mr-8 bg-zinc-900" : "border border-zinc-800"}`}>
    {children}
  </article>;
}
