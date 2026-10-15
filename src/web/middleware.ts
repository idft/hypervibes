import type { IncomingMessage, ServerResponse } from "node:http";
import type { Connect } from "vite";

/** Resolves whether middleware sent a response, including streaming/disconnects. */
export function runMiddleware(middleware: Connect.NextHandleFunction, request: IncomingMessage, response: ServerResponse): Promise<boolean> {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      response.off("finish", handled);
      response.off("close", handled);
    };
    const handled = () => { cleanup(); resolve(true); };
    response.once("finish", handled);
    response.once("close", handled);
    try {
      middleware(request, response, (error?: unknown) => {
        cleanup();
        if (error) reject(error);
        else resolve(false);
      });
    } catch (error) {
      cleanup();
      reject(error);
    }
  });
}
