import { realpath } from "node:fs/promises";
import lockfile from "proper-lockfile";

/** Heartbeat-backed exclusive ownership; stale locks from crashed hosts expire. */
export async function lockProfile(path: string, onCompromised: (error: Error) => void) {
  return lockfile.lock(await realpath(path), {
    realpath: false,
    stale: 30_000,
    update: 5_000,
    retries: 0,
    onCompromised,
  });
}
