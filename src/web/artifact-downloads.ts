import type { FastifyInstance } from "fastify";
import type { AppRuntime } from "../runtime.js";
import { currentUser } from "../auth-user.js";
import { AgentError } from "../agents/agent-input.js";

export function artifactDownloads(app: FastifyInstance, runtime: AppRuntime) {
  app.get<{ Params: { agentId: string; id: string }; Querystring: { inline?: string } }>("/api/agents/:agentId/artifacts/:id/content", async (request, reply) => {
    reply.header("Cache-Control", "no-store").header("X-Content-Type-Options", "nosniff").header("Content-Security-Policy", "sandbox; default-src 'none'; base-uri 'none'");
    const user = await currentUser(runtime, new Headers({ cookie: request.headers.cookie ?? "" }));
    if (!user) return reply.code(401).send({ error: "Authentication required" });
    try {
      await runtime.agents.owned(user.id, request.params.agentId);
      const item = await runtime.agents.research.artifacts.get(request.params.agentId, request.params.id);
      const extension = ({ "text/markdown": "md", "text/plain": "txt", "text/csv": "csv", "application/json": "json", "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp" } as Record<string, string>)[item.mediaType] ?? "bin";
      const filename = `${item.title.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 80)}.${extension}`;
      const disposition = request.query.inline === "1" && item.mediaType.startsWith("image/") ? "inline" : "attachment";
      return reply.type(item.mediaType).header("Content-Disposition", `${disposition}; filename="${filename}"`).send(item.payload);
    } catch (error) {
      if (error instanceof AgentError) return reply.code(error.status).send({ error: error.message });
      throw error;
    }
  });
}
