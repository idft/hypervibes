import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ModuleKind, ScriptTarget, transpileModule } from "typescript";
import { DocumentationService } from "./documentation-service.js";

let directory: string;
let docs: string;
let service: DocumentationService;

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), "hypervibes-documentation-"));
  docs = join(directory, "docs");
  service = new DocumentationService(pathToFileURL(docs + sep));
  await mkdir(docs);
  await writeFile(join(docs, "Alpha.md"), "# Alpha guide\n\nTrading is disabled.\nLiteral [a.*] text.\n");
  await writeFile(join(docs, "index.md"), "# User guide\n\nTrading tools require permission.\n");
});
afterEach(async () => { await rm(directory, { recursive: true, force: true }); });

describe("published documentation", () => {
  it("lists only public, regular, top-level Markdown with stable IDs and titles", async () => {
    for (const [name, content] of Object.entries({
      "Draft.md": "---\ndraft: true\n---\n# Draft secret",
      "Unlisted.md": "---\r\nunlisted: true # hidden\r\n---\r\n# Hidden secret",
      "Invalid.md": "---\ndraft: true\n# Malformed secret",
      "_Internal.md": "# Internal secret",
      ".hidden.md": "# Hidden secret",
      "Script.ts": "secret",
      "Page.mdx": "secret",
    })) await writeFile(join(docs, name), content);
    await mkdir(join(docs, "nested"));
    await writeFile(join(docs, "nested", "Secret.md"), "nested secret");
    await writeFile(join(directory, "Secret.md"), "outside secret");
    await symlink(join(directory, "Secret.md"), join(docs, "Link.md"));
    const result = await service.list({ limit: 1 });
    expect(result).toMatchObject({ documents: [{ id: "Alpha", title: "Alpha guide", path: "docs/Alpha.md" }], total: 2, nextOffset: 1 });
    expect(await service.list({ offset: 1 })).toMatchObject({ documents: [{ id: "index" }], nextOffset: null });
    expect(await service.search({ query: "secret" })).toMatchObject({ matches: [], total: 0 });
  });

  it("reads complete Markdown through bounded character pages", async () => {
    const markdown = "---\nslug: /alpha\n---\n\n# Alpha guide\n\n" + "🛠️ Testing.\n".repeat(1400);
    await writeFile(join(docs, "Alpha.md"), markdown);
    let content = "";
    let offset: number | null = 0;
    while (offset !== null) {
      const page = await service.read({ id: "Alpha", offset, limit: 777 });
      expect(page.content.length).toBeLessThanOrEqual(777);
      content += page.content;
      offset = page.nextOffset;
    }
    expect(content).toBe(markdown);
    expect((await service.read({ id: "Alpha", offset: markdown.length })).content).toBe("");
    await expect(service.read({ id: "Alpha", offset: markdown.length + 1 })).rejects.toThrow("offset exceeds");
  });

  it("rejects traversal, paths, URLs, unknown IDs, and encoded paths", async () => {
    for (const id of ["../Secret", "/etc/passwd", "file:///etc/passwd", "docs/Alpha.md", "Alpha.md", "..\\Secret", "%2e%2e%2fSecret", "Alpha\0", "Missing"]) {
      await expect(service.read({ id })).rejects.toThrow("Unknown documentation ID");
    }
  });

  it("searches literal substrings case-insensitively, with line numbers and pagination", async () => {
    expect(await service.search({ query: "TRADING", limit: 1 })).toEqual({ matches: [{ id: "Alpha", title: "Alpha guide", line: 3, snippet: "Trading is disabled." }], total: 2, nextOffset: 1 });
    expect(await service.search({ query: "trading", offset: 1, limit: 1 })).toMatchObject({ matches: [{ id: "index", line: 3 }], total: 2, nextOffset: null });
    expect(await service.search({ query: "[a.*]" })).toMatchObject({ matches: [{ id: "Alpha", line: 4 }], total: 1 });
    expect(await service.search({ query: "does not exist" })).toEqual({ matches: [], total: 0, nextOffset: null });
    await writeFile(join(docs, "Long.md"), "# Long\n" + "x".repeat(1000) + " NEEDLE " + "y".repeat(1000));
    const long = await new DocumentationService(pathToFileURL(docs + sep)).search({ query: "needle" });
    expect(long.matches[0]!.snippet).toContain("NEEDLE");
    expect(long.matches[0]!.snippet.length).toBeLessThanOrEqual(242);
  });

  it("validates bounds even when called outside the tool schemas", async () => {
    await expect(service.list({ limit: 101 })).rejects.toThrow("limit");
    await expect(service.list({ offset: -1 })).rejects.toThrow("offset");
    await expect(service.read({ id: "Alpha", limit: 12001 })).rejects.toThrow("limit");
    await expect(service.read({ id: "Alpha", offset: 0.5 })).rejects.toThrow("offset");
    await expect(service.search({ query: "   " })).rejects.toThrow("query");
    await expect(service.search({ query: "x".repeat(201) })).rejects.toThrow("query");
    await expect(service.search({ query: "trading", limit: 51 })).rejects.toThrow("limit");
    await expect(service.search({ query: "trading", offset: Infinity })).rejects.toThrow("offset");
  });

  it("loads lazily and keeps a read-only snapshot without leaking filesystem errors", async () => {
    expect((await service.read({ id: "Alpha" })).title).toBe("Alpha guide");
    await writeFile(join(docs, "Alpha.md"), "replacement");
    expect((await service.read({ id: "Alpha" })).title).toBe("Alpha guide");
    const unavailable = new DocumentationService(pathToFileURL(join(directory, "missing") + sep));
    await expect(unavailable.list()).rejects.toThrow("Published HyperVibes documentation is unavailable. Check the application package.");
  });

  it("loads compiled documentation from an isolated package and unrelated working directory", async () => {
    const compiled = join(directory, "dist", "documentation");
    const working = join(directory, "working");
    await mkdir(compiled, { recursive: true });
    await mkdir(working);
    await writeFile(join(directory, "package.json"), JSON.stringify({ type: "module" }));
    const source = await readFile(new URL("./documentation-service.ts", import.meta.url), "utf8");
    const javascript = transpileModule(source, { compilerOptions: { module: ModuleKind.ESNext, target: ScriptTarget.ES2022 } }).outputText;
    const modulePath = join(compiled, "documentation-service.js");
    await writeFile(modulePath, javascript);
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", `
      import { DocumentationService } from ${JSON.stringify(pathToFileURL(modulePath).href)};
      const documentation = new DocumentationService();
      console.log(JSON.stringify(await documentation.read({ id: "Alpha" })));
    `], { cwd: working, encoding: "utf8", timeout: 10000 });
    expect(result.status, result.stderr).toBe(0);
    expect(JSON.parse(result.stdout)).toMatchObject({ id: "Alpha", title: "Alpha guide", path: "docs/Alpha.md", nextOffset: null });
  });

  it("resolves source documentation without relying on the working directory", async () => {
    const documentation = new DocumentationService();
    expect((await documentation.list()).documents.some((page) => page.id === "index")).toBe(true);
    expect((await documentation.read({ id: "index" })).content).toContain("HyperVibes user guide");
  });
});
