import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useWindowsUserAgent } from "../shared/testPlatform";
import { useLargestScan } from "./useLargestScan";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage = () => {};
  },
  invoke,
}));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));
vi.mock("../shared/useStopOnUnmount", () => ({ useStopOnUnmount: vi.fn() }));

const scanArgs = () => invoke.mock.calls[0]?.[1];

describe("useLargestScan", () => {
  it("scans from the root of the disk", () => {
    invoke.mockClear().mockResolvedValue(undefined);
    renderHook(() => useLargestScan());
    expect(invoke.mock.calls[0]?.[0]).toBe("scan_largest_files");
    expect(scanArgs()).toMatchObject({ path: "/", limit: 100 });
  });

  describe("on Windows", () => {
    useWindowsUserAgent();

    it("scans every drive through the empty path", () => {
      invoke.mockClear().mockResolvedValue(undefined);
      renderHook(() => useLargestScan());
      expect(scanArgs()).toMatchObject({ path: "", limit: 100 });
    });
  });
});
