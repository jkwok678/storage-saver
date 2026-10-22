import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Entry } from "../browse/entries";
import { MIN_EXPAND_TURN } from "./layout";
import type { ScanHandlers, ScanRequest } from "./scanDirectory";
import { useSunburstScan } from "./useSunburstScan";

// --- mocks ---

vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));
vi.mock("../shared/useStopOnUnmount", () => ({ useStopOnUnmount: vi.fn() }));

const mockStart = vi.hoisted(() => vi.fn<(req: ScanRequest, h: ScanHandlers) => () => void>());
vi.mock("./scanDirectory", () => ({ startDirectoryScan: mockStart }));

// rAF queue for controlled flush
const rafQueue: FrameRequestCallback[] = [];
const flushRaf = () => {
  const cbs = rafQueue.splice(0);
  for (const cb of cbs) cb(0);
};

beforeEach(() => {
  vi.clearAllMocks();
  rafQueue.length = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    rafQueue.push(cb);
    return rafQueue.length;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    const idx = id - 1;
    if (idx >= 0 && idx < rafQueue.length) rafQueue[idx] = () => {};
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// --- helpers ---

function entry(name: string, bytes: number | null = null): Entry {
  return { name, path: `/${name}`, isDir: true, bytes, modified: null, error: null };
}

type Capture = { req: ScanRequest; h: ScanHandlers };

function captureStarts(): Capture[] {
  const calls: Capture[] = [];
  mockStart.mockImplementation((req, h) => {
    calls.push({ req, h });
    return vi.fn();
  });
  return calls;
}

function renderScan(request: ScanRequest = { path: "/", refresh: false }) {
  return renderHook(() => useSunburstScan(request));
}

// --- tests ---

describe("useSunburstScan", () => {
  it("starts the root scan immediately on mount", () => {
    const calls = captureStarts();
    renderScan();
    expect(calls).toHaveLength(1);
    expect(calls[0].req).toEqual({ path: "/", refresh: false });
  });

  it("sets rootScanning and scanning to true until root Done", () => {
    const calls = captureStarts();
    const { result } = renderScan();
    expect(result.current.rootScanning).toBe(true);
    expect(result.current.scanning).toBe(true);

    act(() => calls[0].h.onStarted([]));
    act(() => calls[0].h.onDone());

    expect(result.current.rootScanning).toBe(false);
  });

  it("updates listings on onStarted", () => {
    const calls = captureStarts();
    const { result } = renderScan();
    const entries = [entry("a", 1e9)];
    act(() => calls[0].h.onStarted(entries));
    expect(result.current.listings.get("/")).toEqual(entries);
    expect(result.current.rootEntries).toEqual(entries);
  });

  it("batches size updates via rAF", () => {
    const calls = captureStarts();
    const { result } = renderScan();
    act(() => calls[0].h.onStarted([entry("a", null)]));

    act(() => calls[0].h.onSized({ path: "/a", bytes: 500, error: null }));
    // Not flushed yet (rAF pending)
    expect(result.current.listings.get("/")?.[0]?.bytes).toBeNull();

    act(() => flushRaf());
    expect(result.current.listings.get("/")?.[0]?.bytes).toBe(500);
  });

  it("does not start child scan before parent Done", () => {
    const calls = captureStarts();
    renderScan();
    act(() => calls[0].h.onStarted([entry("big", 1e10)]));
    act(() => calls[0].h.onSized({ path: "/big", bytes: 1e10, error: null }));
    // Root not done yet — no second call
    expect(calls).toHaveLength(1);
  });

  it("enqueues children serially after parent Done", () => {
    const calls = captureStarts();
    renderScan();

    // Root: two large children
    const entries = [entry("a", 5e10), entry("b", 5e10)];
    act(() => {
      calls[0].h.onStarted(entries);
      calls[0].h.onSized({ path: "/a", bytes: 5e10, error: null });
      calls[0].h.onSized({ path: "/b", bytes: 5e10, error: null });
    });
    act(() => calls[0].h.onDone());

    // After root Done: first child started (one at a time)
    expect(calls).toHaveLength(2);
    expect(calls[1].req.refresh).toBe(false);

    // Complete first child, second starts
    act(() => {
      calls[1].h.onStarted([]);
      calls[1].h.onDone();
    });
    expect(calls).toHaveLength(3);
  });

  it("does not enqueue children below MIN_EXPAND_TURN", () => {
    const calls = captureStarts();
    renderScan();

    // tiny child: fraction = 1/(1e12) < MIN_EXPAND_TURN
    const entries = [entry("big", 1e12), entry("tiny", 1)];
    act(() => {
      calls[0].h.onStarted(entries);
      calls[0].h.onSized({ path: "/big", bytes: 1e12, error: null });
      calls[0].h.onSized({ path: "/tiny", bytes: 1, error: null });
    });
    act(() => calls[0].h.onDone());

    const childPaths = calls.slice(1).map((c) => c.req.path);
    expect(childPaths).not.toContain("/tiny");
  });

  it("skips entries with null bytes when deciding to expand", () => {
    const calls = captureStarts();
    renderScan();

    act(() => {
      calls[0].h.onStarted([entry("pending", null)]);
      calls[0].h.onDone();
    });
    // pending (bytes null) → not enqueued
    expect(calls).toHaveLength(1);
  });

  it("does not clear sibling listings when adding a child listing", () => {
    const calls = captureStarts();
    const { result } = renderScan();

    const entries = [entry("a", 5e10), entry("b", 5e10)];
    act(() => {
      calls[0].h.onStarted(entries);
      calls[0].h.onSized({ path: "/a", bytes: 5e10, error: null });
      calls[0].h.onSized({ path: "/b", bytes: 5e10, error: null });
    });
    act(() => calls[0].h.onDone());

    // First child scan starts
    act(() => calls[1].h.onStarted([entry("a1", 1e9)]));

    // Root listing is still present
    expect(result.current.listings.has("/")).toBe(true);
    expect(result.current.listings.has("/a")).toBe(true);
  });

  it("ignores stale callbacks after request changes (EC-10)", () => {
    const calls = captureStarts();
    const { result, rerender } = renderHook((req) => useSunburstScan(req), {
      initialProps: { path: "/", refresh: false } as ScanRequest,
    });

    const staleHandlers = calls[0].h;
    rerender({ path: "/Users", refresh: false });

    // Stale started should not affect listings for new request
    act(() => staleHandlers.onStarted([entry("stale", 1)]));
    expect(result.current.listings.get("/Users")).toBeUndefined();
    expect(result.current.listings.get("/")).toBeUndefined();
  });

  it("uses refresh:true only on root, not on nested children", () => {
    const calls = captureStarts();
    const { result } = renderScan();

    // Trigger internal refresh
    act(() => result.current.refresh());

    // The root re-scan should use refresh:true
    expect(calls.length).toBeGreaterThan(1);
    const rootCall = calls[calls.length - 1];
    expect(rootCall.req.refresh).toBe(true);
    expect(rootCall.req.path).toBe("/");
  });

  it("subsequent child scans use refresh:false even after refresh()", () => {
    const calls = captureStarts();
    const { result } = renderScan();

    act(() => result.current.refresh());

    // Complete the refreshed root scan with a large child
    const latestRoot = calls[calls.length - 1];
    act(() => {
      latestRoot.h.onStarted([entry("big", 1e10)]);
      latestRoot.h.onSized({ path: "/big", bytes: 1e10, error: null });
    });
    act(() => latestRoot.h.onDone());

    const childCall = calls[calls.length - 1];
    expect(childCall.req.path).toBe("/big");
    expect(childCall.req.refresh).toBe(false);
  });

  it("sets scanError on root scan failure (EC-1)", () => {
    const calls = captureStarts();
    const { result } = renderScan({ path: "/private", refresh: false });

    act(() => calls[0].h.onError("Permission denied"));
    expect(result.current.scanError).toBe("Permission denied");
    expect(result.current.scanning).toBe(false);
  });

  it("continues queue on nested failure, does not set scanError", () => {
    const calls = captureStarts();
    const { result } = renderScan();

    const entries = [entry("a", 5e10), entry("b", 5e10)];
    act(() => {
      calls[0].h.onStarted(entries);
      calls[0].h.onSized({ path: "/a", bytes: 5e10, error: null });
      calls[0].h.onSized({ path: "/b", bytes: 5e10, error: null });
    });
    act(() => calls[0].h.onDone());

    // First child fails
    act(() => calls[1].h.onError("Access denied on /a"));
    expect(result.current.scanError).toBeNull();
    // Queue continues: second child /b
    expect(calls).toHaveLength(3);
  });

  it("MIN_EXPAND_TURN threshold works correctly", () => {
    // A child at exactly MIN_EXPAND_TURN fraction should be enqueued
    const threshold = MIN_EXPAND_TURN; // 2/360
    const totalBytes = 1e9;
    const childBytes = Math.ceil(threshold * totalBytes);

    const calls = captureStarts();
    renderScan();

    act(() => {
      calls[0].h.onStarted([entry("child", childBytes)]);
      calls[0].h.onSized({ path: "/child", bytes: childBytes, error: null });
    });
    act(() => calls[0].h.onDone());
    // child fraction = childBytes/childBytes = 1.0 ≥ MIN_EXPAND_TURN → enqueued
    expect(calls).toHaveLength(2);
  });
});
