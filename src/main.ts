import { pathToFileURL } from "node:url";
import { loadConfig } from "./config.js";
import { createApp } from "./web/app.js";

export async function main(args: string[] = process.argv.slice(2)): Promise<void> {
  const config = loadConfig();
  const force = new AbortController();
  const app = await createApp(config, { logger: true, development: args.includes("--dev"), forceShutdown: force.signal });
  let closing = false;
  const shutdown = () => {
    if (closing) {
      // A second signal forces shutdown past the wait for in-flight agent jobs.
      app.log.info("Second shutdown signal received; forcing shutdown");
      force.abort();
      return;
    }
    closing = true;
    app.log.info("Shutdown signal received; waiting for in-flight agent jobs to complete");
    void app.shutdown().then(() => {
      app.log.info("Shutdown complete");
    }).catch((error: unknown) => {
      app.log.error(error, "Shutdown failed");
      process.exitCode = 1;
    }).finally(removeSignals);
  };
  const removeSignals = () => {
    process.off("SIGINT", shutdown);
    process.off("SIGTERM", shutdown);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  try {
    await app.listen({ host: config.host, port: config.port });
  } catch (error) {
    removeSignals();
    await app.close();
    throw error;
  }
}

// Importing this module in tests or a future desktop host never starts a server.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    console.error(error);
    process.exitCode = 1;
  });
}
