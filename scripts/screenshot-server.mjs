import { readFile, readdir, realpath, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, relative, sep } from "node:path";

const contentTypes = new Map([
  [".js", "text/javascript; charset=utf-8"], [".css", "text/css; charset=utf-8"],
  [".png", "image/png"], [".svg", "image/svg+xml"], [".ico", "image/x-icon"],
  [".jpg", "image/jpeg"], [".jpeg", "image/jpeg"], [".webp", "image/webp"], [".gif", "image/gif"],
  [".woff", "font/woff"], [".woff2", "font/woff2"],
]);
const policy = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

// Read the built files once. No request can read a source file or follow a symlink.
export async function loadScreenshotAssets(directory) {
  const root = await realpath(directory);
  const assets = new Map();
  async function visit(path) {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) throw new Error("Screenshot assets must not contain symlinks");
      const file = join(path, entry.name);
      if (entry.isDirectory()) { await visit(file); continue; }
      if (!entry.isFile()) throw new Error("Screenshot assets must be regular files");
      const name = relative(root, file).split(sep).join("/");
      const url = name === "index.html" ? "/screenshot" : `/${name}`;
      const type = name === "index.html" ? "text/html; charset=utf-8" : contentTypes.get(extname(name));
      if (!type || (name !== "index.html" && !name.startsWith("assets/") && !name.startsWith("static/") && name !== "currency/BTC.svg")) {
        throw new Error(`Unexpected screenshot build file: ${name}`);
      }
      if ((await stat(file)).size > 32 * 1024 * 1024) throw new Error(`Screenshot asset is too large: ${name}`);
      assets.set(url, { body: await readFile(file), type });
    }
  }
  await visit(root);
  if (!assets.has("/screenshot")) throw new Error("Screenshot build has no index.html");
  return assets;
}

export function canonicalRequestPath(url) {
  if (!url || !url.startsWith("/") || url.startsWith("//") || /[\\%#\s]/.test(url)) return null;
  const path = url.split("?", 1)[0];
  if (path.split("/").some((segment, index) => segment === "." || segment === ".." || (index > 0 && !segment))) return null;
  return path;
}

export async function startScreenshotServer(directory) {
  const assets = await loadScreenshotAssets(directory);
  const server = createServer((request, response) => {
    response.setHeader("Content-Security-Policy", policy);
    response.setHeader("X-Content-Type-Options", "nosniff");
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("Referrer-Policy", "no-referrer");
    if (request.method !== "GET" && request.method !== "HEAD") {
      response.writeHead(405, { Allow: "GET, HEAD" }).end();
      return;
    }
    const address = server.address();
    if (request.headers.host !== `127.0.0.1:${address.port}` || request.headers.authorization || request.headers.cookie) {
      response.writeHead(403).end();
      return;
    }
    const path = canonicalRequestPath(request.url);
    const asset = path && assets.get(path);
    if (!asset) { response.writeHead(404).end(); return; }
    response.writeHead(200, { "Content-Type": asset.type, "Content-Length": asset.body.length });
    response.end(request.method === "HEAD" ? undefined : asset.body);
  });
  server.on("upgrade", (_request, socket) => socket.destroy());
  server.on("clientError", (_error, socket) => socket.destroy());
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", () => { server.off("error", reject); resolve(); });
    });
  } catch (error) { server.close(); throw error; }
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    assetPaths: new Set(assets.keys()),
    close: async () => {
      const closed = new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
      server.closeAllConnections();
      await closed;
    },
  };
}
