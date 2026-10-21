import { useSuspenseQuery } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import { createTestQueryClient, withTestQueryClient } from "./query.js";

describe("Query test infrastructure", () => {
  it("isolates caches and applies defaults without discarding test overrides", () => {
    const first = createTestQueryClient({ staleTime: Infinity });
    const second = createTestQueryClient({ retry: 2, retryDelay: 0 });
    first.setQueryData(["fixture"], "first test");
    expect(second.getQueryData(["fixture"])).toBeUndefined();
    expect(first.getDefaultOptions().queries).toMatchObject({ retry: false, gcTime: Infinity, staleTime: Infinity });
    expect(first.getDefaultOptions().mutations?.retry).toBe(false);
    expect(second.getDefaultOptions().queries).toMatchObject({ retry: 2, retryDelay: 0 });
  });

  it("automatically clears query and mutation caches after UI teardown", async () => {
    // Finished callbacks run in reverse order: verify after the factory cleanup.
    onTestFinished(() => {
      expect(client.getQueryCache().getAll()).toHaveLength(0);
      expect(client.getMutationCache().getAll()).toHaveLength(0);
    });
    const client = createTestQueryClient();
    client.setQueryData(["fixture"], "seed");
    await client.getMutationCache().build(client, { mutationFn: async () => "saved" }).execute(undefined);
    onTestFinished(() => { expect(client.getQueryData(["fixture"])).toBe("seed"); });
  });

  it("wraps repeated renders with the same client without resetting cached data", () => {
    const client = createTestQueryClient({ staleTime: Infinity });
    const fetch = vi.fn();
    function Probe() {
      const { data } = useSuspenseQuery({ queryKey: ["fixture"], queryFn: fetch });
      return <span>{data}</span>;
    }
    const Wrapped = withTestQueryClient(Probe, () => client);
    client.setQueryData(["fixture"], "seed");
    expect(renderToStaticMarkup(<Wrapped />)).toBe("<span>seed</span>");
    client.setQueryData(["fixture"], "saved");
    expect(renderToStaticMarkup(<Wrapped />)).toBe("<span>saved</span>");
    expect(fetch).not.toHaveBeenCalled();
  });
});
