import { execFileSync } from "node:child_process";

type Git = (args: string[]) => string;

function runGit(args: string[]): string {
  return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
}

/** Resolve the label once at build time, not when the application runs. */
export function buildLabel(command: "serve" | "build", git: Git = runGit): string {
  if (command === "serve") return "Development";

  try {
    const dirty = git(["status", "--porcelain"]).length > 0;
    if (!dirty) {
      const tags = git(["tag", "--points-at", "HEAD", "--list", "v*"])
        .split("\n")
        .filter((tag) => /^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag))
        .sort();
      if (tags.length > 0) return tags[0]!;
    }
    const commit = git(["rev-parse", "--short", "HEAD"]);
    return /^[0-9a-f]+$/.test(commit) ? `${commit}${dirty ? "-dirty" : ""}` : "Unknown build";
  } catch {
    return "Unknown build";
  }
}
