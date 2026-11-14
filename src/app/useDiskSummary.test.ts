import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useDiskSummary } from "./useDiskSummary";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));

const summary = (total: number) => ({ total, used: 1, available: total - 1 });

describe("useDiskSummary", () => {
  it("reads the given path and drops the result when the path changes", async () => {
    invoke.mockImplementation((_cmd: string, args: { path: string }) =>
      Promise.resolve(summary(args.path === "C:\\" ? 100 : 200)),
    );
    const { result, rerender } = renderHook(({ path }) => useDiskSummary(path), {
      initialProps: { path: "C:\\" },
    });
    await waitFor(() => expect(result.current.summary).toEqual(summary(100)));
    expect(invoke).toHaveBeenLastCalledWith("disk_summary", { path: "C:\\" });

    rerender({ path: "D:\\" });
    expect(result.current.summary).toBeNull();
    await waitFor(() => expect(result.current.summary).toEqual(summary(200)));
    expect(invoke).toHaveBeenLastCalledWith("disk_summary", { path: "D:\\" });
  });

  it("reports a failed read", async () => {
    invoke.mockRejectedValue("No access");
    const { result } = renderHook(() => useDiskSummary("/"));
    await waitFor(() => expect(result.current.error).toBe("No access"));
  });
});
