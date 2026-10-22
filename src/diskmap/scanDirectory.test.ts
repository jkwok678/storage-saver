import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Entry } from "../browse/entries";
import { type ScanHandlers, type ScanRequest, startDirectoryScan } from "./scanDirectory";

const mockInvoke = vi.hoisted(() => vi.fn());
const channelStore = vi.hoisted(() => ({
  last: null as { onmessage?: (msg: unknown) => void } | null,
}));

vi.mock("@tauri-apps/api/core", () => {
  class Channel {
    onmessage?: (msg: unknown) => void;
    constructor() {
      channelStore.last = this;
    }
  }
  return { Channel, invoke: mockInvoke };
});

function entry(name: string): Entry {
  return { name, path: `/${name}`, isDir: false, bytes: null, modified: null, error: null };
}

function makeHandlers(
  overrides: Partial<ScanHandlers> = {},
): ScanHandlers & { started: Entry[][]; sized: unknown[]; done: number; errors: string[] } {
  const started: Entry[][] = [];
  const sized: unknown[] = [];
  let done = 0;
  const errors: string[] = [];
  return {
    isCurrent: () => true,
    onStarted: (e) => started.push(e),
    onSized: (u) => sized.push(u),
    onDone: () => {
      done++;
    },
    onError: (m) => errors.push(m),
    ...overrides,
    get started() {
      return started;
    },
    get sized() {
      return sized;
    },
    get done() {
      return done;
    },
    get errors() {
      return errors;
    },
  };
}

const request: ScanRequest = { path: "/Users", refresh: false };

describe("startDirectoryScan", () => {
  beforeEach(() => {
    channelStore.last = null;
    mockInvoke.mockResolvedValue(undefined);
  });

  it("forwards started / sized / done channel events in order", () => {
    const h = makeHandlers();
    startDirectoryScan(request, h);

    const ch = channelStore.last;
    expect(ch?.onmessage).toBeTruthy();
    const entries = [entry("a"), entry("b")];
    ch?.onmessage?.({ event: "started", data: { entries } });
    ch?.onmessage?.({ event: "sized", data: { path: "/a", bytes: 100, error: null } });
    ch?.onmessage?.({ event: "done" });

    expect(h.started).toEqual([entries]);
    expect(h.sized).toEqual([{ path: "/a", bytes: 100, error: null }]);
    expect(h.done).toBe(1);
  });

  it("ignores all events after cancel is called", () => {
    const h = makeHandlers();
    const cancel = startDirectoryScan(request, h);
    cancel();

    channelStore.last?.onmessage?.({ event: "started", data: { entries: [] } });
    channelStore.last?.onmessage?.({ event: "done" });

    expect(h.started).toHaveLength(0);
    expect(h.done).toBe(0);
  });

  it("ignores events when isCurrent returns false", () => {
    let current = true;
    const h = makeHandlers({ isCurrent: () => current });
    startDirectoryScan(request, h);

    current = false;
    channelStore.last?.onmessage?.({ event: "started", data: { entries: [] } });

    expect(h.started).toHaveLength(0);
  });

  it("calls onError with errorText when invoke rejects", async () => {
    mockInvoke.mockRejectedValue("permission denied");
    const h = makeHandlers();
    startDirectoryScan(request, h);

    await Promise.resolve(); // let microtask queue flush
    expect(h.errors).toEqual(["permission denied"]);
    expect(h.done).toBe(0);
  });

  it("treats invoke resolve as done when no done event arrives", async () => {
    const h = makeHandlers();
    startDirectoryScan(request, h);
    await Promise.resolve();
    expect(h.done).toBe(1);
  });

  it("does not call onError after cancel", async () => {
    mockInvoke.mockRejectedValue("err");
    const h = makeHandlers();
    const cancel = startDirectoryScan(request, h);
    cancel();

    await Promise.resolve();
    expect(h.errors).toHaveLength(0);
  });
});
