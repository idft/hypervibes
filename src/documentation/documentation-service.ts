import { readdir, readFile } from "node:fs/promises";

export interface DocumentationPage {
  id: string;
  title: string;
  path: string;
  characters: number;
}

interface Document extends DocumentationPage {
  content: string;
}

const publishedFilename = /^[A-Za-z][A-Za-z0-9_-]*\.md$/;
const defaultDirectory = new URL("../../docs/", import.meta.url);

function boundedInteger(value: number | undefined, fallback: number, min: number, max: number, name: string): number {
  const result = value ?? fallback;
  if (!Number.isSafeInteger(result) || result < min || result > max) {
    throw new Error(`${name} must be an integer from ${min} to ${max}`);
  }
  return result;
}

function isPublished(content: string): boolean {
  if (!content.startsWith("---\n") && !content.startsWith("---\r\n")) return true;
  const frontmatter = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/.exec(content);
  if (!frontmatter) return false;
  return !/^\s*(?:draft|unlisted)\s*:\s*(?:true|"true"|'true')\s*(?:#.*)?$/im.test(frontmatter[1]!);
}

function metadata(document: Document): DocumentationPage {
  return { id: document.id, title: document.title, path: document.path, characters: document.characters };
}

/** Read-only snapshot of the published, top-level docs/*.md corpus.
 * The directory is a host-only dependency for tests; no tool accepts a path.
 * Source and compiled modules have the same depth below the package root.
 * Construction performs no I/O. The first request loads the snapshot.
 */
export class DocumentationService {
  private documents: Promise<Document[]> | undefined;

  constructor(private readonly directory: URL = defaultDirectory) {}

  private load(): Promise<Document[]> {
    this.documents ??= this.loadPublishedDocuments();
    return this.documents;
  }

  private async loadPublishedDocuments(): Promise<Document[]> {
    try {
      const entries = await readdir(this.directory, { withFileTypes: true });
      const documents: Document[] = [];
      for (const entry of entries.sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
        // Do not follow symlinks, subdirectories, hidden files, or developer files.
        if (!entry.isFile() || !publishedFilename.test(entry.name)) continue;
        const content = await readFile(new URL(entry.name, this.directory), "utf8");
        if (!isPublished(content)) continue;
        const id = entry.name.slice(0, -3);
        const title = /^#\s+(.+?)\s*#*\s*$/m.exec(content)?.[1]?.trim() ?? id;
        documents.push({ id, title, path: `docs/${entry.name}`, characters: content.length, content });
      }
      return documents;
    } catch {
      // Do not expose deployment paths or filesystem errors to the model.
      throw new Error("Published HyperVibes documentation is unavailable. Check the application package.");
    }
  }

  async list(input: { offset?: number; limit?: number } = {}) {
    const offset = boundedInteger(input.offset, 0, 0, Number.MAX_SAFE_INTEGER, "offset");
    const limit = boundedInteger(input.limit, 50, 1, 100, "limit");
    const documents = await this.load();
    const items = documents.slice(offset, offset + limit).map(metadata);
    return { documents: items, total: documents.length, nextOffset: offset + items.length < documents.length ? offset + items.length : null };
  }

  async read(input: { id: string; offset?: number; limit?: number }) {
    // IDs are opaque catalog identifiers, never filesystem paths or URLs.
    if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(input.id)) throw new Error("Unknown documentation ID. Use list_docs to find an exact ID.");
    const offset = boundedInteger(input.offset, 0, 0, Number.MAX_SAFE_INTEGER, "offset");
    const limit = boundedInteger(input.limit, 8000, 1, 12000, "limit");
    const document = (await this.load()).find((candidate) => candidate.id === input.id);
    if (!document) throw new Error("Unknown documentation ID. Use list_docs to find an exact ID.");
    if (offset > document.characters) throw new Error("offset exceeds the document length");
    const content = document.content.slice(offset, offset + limit);
    return { ...metadata(document), content, offset, nextOffset: offset + content.length < document.characters ? offset + content.length : null };
  }

  async search(input: { query: string; offset?: number; limit?: number }) {
    const query = input.query.trim();
    if (!query || query.length > 200) throw new Error("query must contain 1 to 200 characters");
    const offset = boundedInteger(input.offset, 0, 0, Number.MAX_SAFE_INTEGER, "offset");
    const limit = boundedInteger(input.limit, 20, 1, 50, "limit");
    const needle = query.toLowerCase();
    const matches: { id: string; title: string; line: number; snippet: string }[] = [];
    for (const document of await this.load()) {
      for (const [index, line] of document.content.split(/\r?\n/).entries()) {
        const position = line.toLowerCase().indexOf(needle);
        if (position === -1) continue;
        const start = Math.max(0, position - 80);
        const end = Math.min(line.length, start + 240);
        matches.push({ id: document.id, title: document.title, line: index + 1, snippet: `${start ? "…" : ""}${line.slice(start, end)}${end < line.length ? "…" : ""}` });
      }
    }
    const items = matches.slice(offset, offset + limit);
    return { matches: items, total: matches.length, nextOffset: offset + items.length < matches.length ? offset + items.length : null };
  }
}
