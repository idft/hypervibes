import { describe, expect, it, vi } from "vitest";
import { buildLabel } from "./build-label.js";

describe("build label", () => {
  it("does not call Git in development", () => {
    const git = vi.fn();
    expect(buildLabel("serve", git)).toBe("Development");
    expect(git).not.toHaveBeenCalled();
  });

  it("uses an exact release tag", () => {
    const git = vi.fn((args: string[]) => args[0] === "status" ? "" : "other\nv0.4.0\n");
    expect(buildLabel("build", git)).toBe("v0.4.0");
    expect(git).toHaveBeenCalledTimes(2);
  });

  it("uses the commit when no release tag exists", () => {
    const git = vi.fn((args: string[]) => args[0] === "status" ? "" : args[0] === "tag" ? "v-not-a-release" : "a815afd");
    expect(buildLabel("build", git)).toBe("a815afd");
  });

  it("marks a dirty production checkout instead of claiming a release", () => {
    const git = vi.fn((args: string[]) => args[0] === "status" ? " M file" : "a815afd");
    expect(buildLabel("build", git)).toBe("a815afd-dirty");
  });

  it("handles missing Git and invalid output", () => {
    expect(buildLabel("build", () => { throw new Error("Git unavailable"); })).toBe("Unknown build");
    expect(buildLabel("build", (args) => args[0] === "rev-parse" ? "not a hash" : "")).toBe("Unknown build");
  });
});
