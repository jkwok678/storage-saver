import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiProviderId, AiProviderStatus } from "./aiProviders";
import { useModelPicker } from "./useModelPicker";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));

const status = (id: AiProviderId, connected: boolean): AiProviderStatus => ({
  id,
  name: id,
  installed: true,
  connected,
});

const models = [{ id: "opus", label: "Opus" }];

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe("useModelPicker", () => {
  it("lists models once signed in and starts at the CLI default", async () => {
    invoke.mockResolvedValue(models);
    const { result, rerender } = renderHook(
      ({ connected }) => useModelPicker("claude", status("claude", connected)),
      { initialProps: { connected: false } },
    );
    expect(result.current.choices).toBeUndefined();
    expect(invoke).not.toHaveBeenCalled();

    rerender({ connected: true });
    await waitFor(() => expect(result.current.choices).toHaveLength(2));
    expect(invoke).toHaveBeenCalledWith("ai_models", { provider: "claude" });
    expect(result.current.modelChoice?.key).toBe("default");
    expect(result.current.effortChoice?.label).toBe("Default");
  });

  it("remembers the pick per provider", async () => {
    invoke.mockResolvedValue(models);
    const first = renderHook(() => useModelPicker("claude", status("claude", true)));
    await waitFor(() => expect(first.result.current.choices).toBeDefined());
    act(() => first.result.current.updatePick({ model: "opus", effort: "High" }));
    expect(first.result.current.effortChoice).toEqual({
      label: "High",
      model: "opus",
      effort: "high",
    });
    first.unmount();

    const second = renderHook(() => useModelPicker("claude", status("claude", true)));
    expect(second.result.current.pick).toEqual({ model: "opus", effort: "High" });
  });

  it("ignores a stored pick it cannot read", () => {
    localStorage.setItem("aiAdvisor.model.claude", "{not json");
    localStorage.setItem("aiAdvisor.model.cursor", JSON.stringify({ model: 3, effort: "Low" }));
    const { result, rerender } = renderHook(({ id }) => useModelPicker(id, status(id, false)), {
      initialProps: { id: "claude" as AiProviderId },
    });
    expect(result.current.pick).toEqual({ model: null, effort: null });
    rerender({ id: "cursor" });
    expect(result.current.pick).toEqual({ model: null, effort: "Low" });
  });

  it("shows a listing failure and tries again on the next visit", async () => {
    invoke.mockRejectedValueOnce("Not signed in").mockResolvedValueOnce(models);
    const { result, rerender } = renderHook(({ id }) => useModelPicker(id, status(id, true)), {
      initialProps: { id: "cursor" as AiProviderId },
    });
    await waitFor(() => expect(result.current.catalogError).toBe("Not signed in"));
    rerender({ id: "claude" });
    rerender({ id: "cursor" });
    await waitFor(() => expect(result.current.catalogError).toBeUndefined());
    expect(invoke).toHaveBeenCalledTimes(3);
  });
});
