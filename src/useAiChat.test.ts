import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { modelChoices } from "./aiModels";
import type { AiProviderStatus } from "./aiProviders";
import type { AiQueryEvent } from "./aiTypes";
import { useAiChat } from "./useAiChat";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));
const app = vi.hoisted(() => ({ running: true }));
vi.mock("./format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./format")>()),
  runningInApp: () => app.running,
}));

const claude: AiProviderStatus = {
  id: "claude",
  name: "Claude Code",
  installed: true,
  connected: true,
};
const [, opus] = modelChoices("claude", [{ id: "opus", label: "Opus" }]);

function renderChat() {
  return renderHook(() =>
    useAiChat({
      provider: "claude",
      activeProvider: claude,
      currentPath: "/Users/me",
      modelChoice: opus,
      effortChoice: opus.efforts[2],
    }),
  );
}

/** Holds `query_ai` open and hands back its channel, to stream events in by hand. */
function pendingQuery() {
  let finish: (value?: unknown) => void = () => {};
  let fail: (reason: unknown) => void = () => {};
  invoke.mockImplementation((command: string) =>
    command === "query_ai"
      ? new Promise((resolve, reject) => {
          finish = resolve;
          fail = reject;
        })
      : Promise.resolve(),
  );
  const emit = (event: AiQueryEvent) => {
    const call = invoke.mock.calls.filter(([command]: unknown[]) => command === "query_ai").pop();
    act(() => call?.[1].onEvent.onmessage(event));
  };
  return { emit, finish: () => finish(), fail: (reason: unknown) => fail(reason) };
}

beforeEach(() => {
  vi.clearAllMocks();
  app.running = true;
});

describe("useAiChat", () => {
  it("streams a reply and its recommendations", async () => {
    const query = pendingQuery();
    const { result } = renderChat();
    act(() => result.current.setInput("  What can go?  "));
    let sent: Promise<void> = Promise.resolve();
    act(() => {
      sent = result.current.sendMessage(result.current.input);
    });

    expect(result.current.input).toBe("");
    expect(result.current.querying).toBe(true);
    expect(result.current.statusMessage).toBe("Gathering storage data…");
    expect(invoke).toHaveBeenCalledWith("query_ai", {
      provider: "claude",
      prompt: "What can go?",
      currentPath: "/Users/me",
      history: [],
      model: "opus",
      effort: "medium",
      onEvent: expect.anything(),
    });
    const [user, reply] = result.current.messages;
    expect(user).toMatchObject({ role: "user", content: "What can go?" });
    expect(reply).toMatchObject({ providerName: "Claude Code", modelLabel: "Opus · Medium" });

    query.emit({ event: "status", data: { message: "Reading folders…" } });
    expect(result.current.statusMessage).toBe("Reading folders…");
    query.emit({ event: "chunk", data: { text: "Partial" } });
    expect(result.current.statusMessage).toBeNull();
    expect(result.current.messages[1].content).toBe("Partial");

    const rec = {
      path: "/Users/me/cache",
      name: "cache",
      isDir: true,
      bytes: 10,
      safety: "safe" as const,
      reason: "Rebuilt on demand",
      exists: true,
    };
    query.emit({ event: "done", data: { answer: "Done", recommendations: [rec] } });
    expect(result.current.querying).toBe(false);
    expect(result.current.messages[1]).toMatchObject({ content: "Done", recommendations: [rec] });

    query.finish();
    await act(() => sent);
  });

  it("sends earlier answers, not failed ones, as history", async () => {
    const query = pendingQuery();
    const { result } = renderChat();
    act(() => {
      result.current.sendMessage("first");
    });
    query.emit({ event: "done", data: { answer: "Answer", recommendations: [] } });
    query.finish();
    await act(async () => {});

    const failing = pendingQuery();
    let second: Promise<void> = Promise.resolve();
    act(() => {
      second = result.current.sendMessage("second");
    });
    failing.fail("Rate limited");
    await act(() => second);
    expect(result.current.queryError).toBe("Rate limited");
    expect(result.current.messages[3]).toMatchObject({
      content: "Could not query Claude Code: Rate limited",
      outcome: "error",
    });

    pendingQuery();
    act(() => {
      result.current.sendMessage("third");
    });
    expect(invoke.mock.lastCall?.[1].history).toEqual([
      { role: "user", content: "first" },
      { role: "assistant", content: "Answer" },
      { role: "user", content: "second" },
    ]);
  });

  it("stops a reply and ignores what arrives after", () => {
    const query = pendingQuery();
    const { result } = renderChat();
    act(() => {
      result.current.sendMessage("Go");
    });
    act(() => result.current.stop());
    expect(invoke).toHaveBeenCalledWith("stop_ai_query");
    expect(result.current.querying).toBe(false);
    expect(result.current.messages[1].outcome).toBe("stopped");
    query.emit({ event: "chunk", data: { text: "late" } });
    expect(result.current.messages[1].content).toBe("");

    act(() => result.current.clear());
    expect(result.current.messages).toEqual([]);
  });

  it("stops the CLI when unmounted mid-reply", () => {
    pendingQuery();
    const { result, unmount } = renderChat();
    act(() => {
      result.current.sendMessage("Go");
    });
    unmount();
    expect(invoke).toHaveBeenCalledWith("stop_ai_query");
  });

  it("ignores blank prompts and needs the app", async () => {
    const { result } = renderChat();
    await act(() => result.current.sendMessage("   "));
    expect(result.current.messages).toEqual([]);
    app.running = false;
    await act(() => result.current.sendMessage("Hi"));
    expect(result.current.queryError).toBe("Storage Saver app is required to query AI.");
    expect(invoke).not.toHaveBeenCalled();
  });
});
