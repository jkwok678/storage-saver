import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTrash } from "./useTrash";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const revealItemInDir = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir }));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));

const outcome = { originalPath: "/a", trashPath: "/.Trash/a", freedBytes: 0, putBack: true };

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useTrash", () => {
  it("moves an item to the Trash and reports it", async () => {
    invoke.mockResolvedValue(outcome);
    const onTrashed = vi.fn();
    const { result } = renderHook(() => useTrash(onTrashed));
    act(() => result.current.setConfirmingTrash("m\n/a"));
    await act(() => result.current.moveToTrash("/a"));
    expect(invoke).toHaveBeenCalledWith("move_to_trash", { path: "/a" });
    expect(result.current.trashedPaths.get("/a")).toEqual(outcome);
    expect(result.current.confirmingTrash).toBeNull();
    expect(result.current.trashingPath).toBeNull();
    expect(onTrashed).toHaveBeenCalledWith("/a", 0);
  });

  it("keeps the failure against its path until cleared", async () => {
    invoke.mockRejectedValue("Protected folder");
    const { result } = renderHook(() => useTrash());
    await act(() => result.current.moveToTrash("/System"));
    expect(result.current.trashError).toEqual({ path: "/System", message: "Protected folder" });
    act(() => result.current.clearTrashError());
    expect(result.current.trashError).toBeNull();
  });

  it("reveals in Finder and shows a reveal failure", async () => {
    revealItemInDir.mockRejectedValue("Gone");
    const { result } = renderHook(() => useTrash());
    act(() => result.current.reveal("/b"));
    expect(revealItemInDir).toHaveBeenCalledWith("/b");
    await waitFor(() => expect(result.current.trashError).toEqual({ path: "/b", message: "Gone" }));
  });
});
