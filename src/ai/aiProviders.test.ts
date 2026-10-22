import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type AiProviderStatus,
  type AiProviders,
  findProvider,
  providerDotClass,
  providerStatusText,
  useAiProviders,
} from "./aiProviders";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const app = vi.hoisted(() => ({ running: true }));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => app.running,
}));

const claude: AiProviderStatus = {
  id: "claude",
  name: "Claude Code",
  installed: true,
  connected: true,
  account: "me@example.com",
};
const cursor: AiProviderStatus = {
  id: "cursor",
  name: "Cursor",
  installed: true,
  connected: false,
};

function providers(overrides: Partial<AiProviders> = {}): AiProviders {
  return {
    providers: [claude],
    checking: false,
    busy: null,
    error: null,
    load: vi.fn(),
    refresh: vi.fn(),
    connect: vi.fn(),
    disconnect: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  app.running = true;
});

describe("findProvider", () => {
  it("returns the checked status, or a placeholder while checking", () => {
    expect(findProvider(providers(), "claude")).toBe(claude);
    expect(findProvider(providers({ providers: null, checking: true }), "cursor")).toEqual({
      id: "cursor",
      name: "Cursor",
      installed: false,
      connected: false,
    });
  });
});

describe("providerStatusText", () => {
  it("describes each sign-in state", () => {
    const ai = providers();
    expect(providerStatusText(ai, claude)).toBe("me@example.com");
    expect(providerStatusText(ai, { ...claude, account: null, detail: "Pro plan" })).toBe(
      "Pro plan",
    );
    expect(providerStatusText(ai, { ...claude, account: null })).toBe("Connected");
    expect(providerStatusText(ai, cursor)).toBe("Not signed in");
    expect(providerStatusText(ai, { ...cursor, installed: false })).toBe("Cursor CLI not found");
    expect(providerStatusText(providers({ checking: true }), cursor)).toBe("Checking connection…");
    app.running = false;
    expect(providerStatusText(ai, claude)).toBe("Open Storage Saver to connect accounts");
  });
});

describe("providerDotClass", () => {
  it("marks connected, signed-out and missing CLIs", () => {
    const ai = providers();
    expect(providerDotClass(ai, claude)).toBe("connected");
    expect(providerDotClass(ai, cursor)).toBe("disconnected");
    expect(providerDotClass(ai, { ...cursor, installed: false })).toBe("missing");
    expect(providerDotClass(providers({ checking: true }), { ...cursor, installed: false })).toBe(
      "disconnected",
    );
  });
});

describe("useAiProviders", () => {
  it("checks the CLIs once on load", async () => {
    invoke.mockResolvedValue([claude]);
    const { result } = renderHook(() => useAiProviders());
    expect(result.current.checking).toBe(true);
    act(() => result.current.load());
    await waitFor(() => expect(result.current.providers).toEqual([claude]));
    act(() => result.current.load());
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("ai_providers_status");
  });

  it("keeps an empty list and the error when the check fails", async () => {
    invoke.mockRejectedValue("CLI crashed");
    const { result } = renderHook(() => useAiProviders());
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.error).toBe("CLI crashed"));
    expect(result.current.providers).toEqual([]);
  });

  it("replaces or adds a provider after connecting", async () => {
    invoke.mockResolvedValueOnce([cursor]);
    const { result } = renderHook(() => useAiProviders());
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.providers).toEqual([cursor]));

    invoke.mockResolvedValueOnce({ ...cursor, connected: true });
    await act(() => result.current.connect("cursor"));
    expect(invoke).toHaveBeenLastCalledWith("connect_ai_provider", { provider: "cursor" });
    expect(result.current.providers).toEqual([{ ...cursor, connected: true }]);

    invoke.mockResolvedValueOnce({ ...claude, connected: false });
    await act(() => result.current.disconnect("claude"));
    expect(result.current.providers).toHaveLength(2);
    expect(result.current.busy).toBeNull();
  });

  it("shows a failed sign-in", async () => {
    invoke.mockRejectedValue("Login cancelled");
    const { result } = renderHook(() => useAiProviders());
    await act(() => result.current.connect("claude"));
    expect(result.current.error).toBe("Login cancelled");
  });

  it("does nothing outside the app", async () => {
    app.running = false;
    const { result } = renderHook(() => useAiProviders());
    act(() => result.current.refresh());
    await act(() => result.current.connect("claude"));
    expect(invoke).not.toHaveBeenCalled();
    expect(result.current.checking).toBe(false);
  });
});
