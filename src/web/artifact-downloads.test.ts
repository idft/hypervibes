import Fastify, { type FastifyInstance } from "fastify";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { currentUser } from "../auth-user.js";
import { artifactDownloads } from "./artifact-downloads.js";
vi.mock("../auth-user.js", () => ({ currentUser: vi.fn() }));

describe("authenticated artifact downloads", () => {
  let directory: string, runtime: AppRuntime, app: FastifyInstance;
  let agentId: string, otherAgentId: string, artifactId: string;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-artifact-http-"));
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite") }));
    await runtime.database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    agentId = randomUUID(); otherAgentId = randomUUID();
    for (const id of [agentId, otherAgentId]) await runtime.database.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES (?,'owner','Agent',?,0,0)", id, randomUUID());
    artifactId = (await runtime.agents.research.artifacts.create(agentId, "fixture", { title: 'Report\r\n"unsafe"', mediaType: "text/markdown", content: "<script>not executable</script>", retained: true })).id;
    vi.mocked(currentUser).mockResolvedValue({ id: "owner", walletAddress: `0x${"1".repeat(40)}` });
    app = Fastify(); artifactDownloads(app, runtime);
  });
  afterEach(async () => { await app.close(); await runtime.close(); rmSync(directory, { recursive: true, force: true }); vi.resetAllMocks(); });
  const url = (agentId: string, artifactId: string) => `/api/agents/${agentId}/artifacts/${artifactId}/content`;
  it("requires authentication and root-Agent authorization, including for another root of the same user", async () => {
    vi.mocked(currentUser).mockResolvedValue(null);
    expect((await app.inject({ url: url(agentId, artifactId) })).statusCode).toBe(401);
    vi.mocked(currentUser).mockResolvedValue({ id: "stranger", walletAddress: "0x0" });
    expect((await app.inject({ url: url(agentId, artifactId) })).statusCode).toBe(404);
    vi.mocked(currentUser).mockResolvedValue({ id: "owner", walletAddress: "0x0" });
    expect((await app.inject({ url: url(otherAgentId, artifactId) })).statusCode).toBe(404);
  });
  it("downloads text as an attachment, sanitizes filenames and disables caching/sniffing", async () => {
    const response = await app.inject({ url: `${url(agentId, artifactId)}?inline=1` });
    expect(response.statusCode).toBe(200); expect(response.body).toBe("<script>not executable</script>");
    expect(response.headers["content-disposition"]).toMatch(/^attachment; filename="[a-zA-Z0-9_.-]+"$/);
    expect(response.headers["cache-control"]).toBe("no-store");
    expect(response.headers["x-content-type-options"]).toBe("nosniff");
    expect(response.headers["content-security-policy"]).toContain("sandbox");
  });
  it("serves allowed image attachments inline, and refuses expired payloads", async () => {
    const item = await runtime.agents.research.artifacts.create(agentId, "image", { title: "Chart", mediaType: "image/png", encoding: "base64", content: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jGJkAAAAASUVORK5CYII=" });
    const response = await app.inject({ url: `${url(agentId, item.id)}?inline=1` });
    expect(response.statusCode).toBe(200); expect(response.headers["content-type"]).toBe("image/png"); expect(response.headers["content-disposition"]).toMatch(/^inline;/);
    await runtime.database.run("UPDATE hv_artifacts SET expires_at=0 WHERE id=?", item.id);
    expect((await app.inject({ url: url(agentId, item.id) })).statusCode).toBe(404);
  });
});
