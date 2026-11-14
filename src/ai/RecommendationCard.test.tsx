import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useWindowsUserAgent } from "../shared/testPlatform";
import type { AiRecommendation, TrashOutcome } from "./aiTypes";
import { RecommendationCard } from "./RecommendationCard";
import type { TrashState } from "./useTrash";

const rec: AiRecommendation = {
  path: "/Users/me/Library/Caches/app",
  name: "app",
  isDir: true,
  bytes: 2048,
  safety: "safe",
  reason: "Rebuilt on demand",
  exists: true,
};

function trashState(overrides: Partial<TrashState> = {}): TrashState {
  return {
    confirmingTrash: null,
    setConfirmingTrash: vi.fn(),
    trashingPath: null,
    trashedPaths: new Map(),
    trashError: null,
    clearTrashError: vi.fn(),
    reveal: vi.fn(),
    moveToTrash: vi.fn(),
    ...overrides,
  };
}

function renderCard(trash: TrashState, overrides: Partial<AiRecommendation> = {}) {
  render(<RecommendationCard rec={{ ...rec, ...overrides }} messageId="a-1" trash={trash} />);
}

const outcome = (freedBytes: number, putBack: boolean): TrashOutcome => ({
  originalPath: rec.path,
  trashPath: "/Users/me/.Trash/app",
  freedBytes,
  putBack,
});

describe("RecommendationCard", () => {
  it("shows the item and offers Finder and Trash", () => {
    const trash = trashState();
    renderCard(trash);
    expect(screen.getByText("2.00 KB")).toBeTruthy();
    expect(screen.getByText("Safe")).toBeTruthy();
    expect(screen.getByText("Rebuilt on demand")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show in Finder" }));
    expect(trash.reveal).toHaveBeenCalledWith(rec.path);
    fireEvent.click(screen.getByRole("button", { name: "Move to Trash" }));
    expect(trash.clearTrashError).toHaveBeenCalled();
    expect(trash.setConfirmingTrash).toHaveBeenCalledWith(`a-1\n${rec.path}`);
  });

  it("confirms before moving to the Trash", () => {
    const trash = trashState({ confirmingTrash: `a-1\n${rec.path}` });
    renderCard(trash, { safety: "caution", bytes: null, reason: "" });
    expect(screen.getByText("Review")).toBeTruthy();
    expect(screen.getByText("Move to Trash?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Move to Trash" }));
    expect(trash.moveToTrash).toHaveBeenCalledWith(rec.path);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(trash.setConfirmingTrash).toHaveBeenCalledWith(null);
  });

  it("disables the confirm buttons while moving", () => {
    renderCard(trashState({ confirmingTrash: `a-1\n${rec.path}`, trashingPath: rec.path }));
    const moving = screen.getByRole("button", { name: "Moving…" }) as HTMLButtonElement;
    expect(moving.disabled).toBe(true);
  });

  it("says when the item is gone or failed to move", () => {
    renderCard(trashState({ trashError: { path: rec.path, message: "Protected" } }), {
      exists: false,
      isDir: false,
    });
    expect(screen.getByText("Not found on disk")).toBeTruthy();
    expect(screen.getByText("Protected")).toBeTruthy();
  });

  it("explains how to free the space and restore it", () => {
    renderCard(trashState({ trashedPaths: new Map([[rec.path, outcome(4096, true)]]) }));
    expect(screen.getByText("Moved to Trash")).toBeTruthy();
    expect(
      screen.getByText("Empty the Trash to free 4.00 KB. Restore it with Put Back in Finder."),
    ).toBeTruthy();
  });

  it("warns when Put Back is unavailable", () => {
    renderCard(trashState({ trashedPaths: new Map([[rec.path, outcome(0, false)]]) }));
    expect(screen.getByText(/Put Back is unavailable/)).toBeTruthy();
  });

  it("counts an item inside a trashed folder as trashed", () => {
    renderCard(
      trashState({ trashedPaths: new Map([["/Users/me/Library/Caches", outcome(0, true)]]) }),
    );
    expect(screen.getByText("Moved to Trash")).toBeTruthy();
    expect(screen.queryByText(/Put Back/)).toBeNull();
  });
});

describe("RecommendationCard on Windows", () => {
  useWindowsUserAgent();

  it("uses File Explorer and Recycle Bin wording", () => {
    const trash = trashState();
    const { unmount } = render(<RecommendationCard rec={rec} messageId="a-1" trash={trash} />);
    expect(screen.getByRole("button", { name: "Show in File Explorer" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Move to Recycle Bin" }).title).toBe(
      "Move this file or folder to the Recycle Bin",
    );
    unmount();
    renderCard(trashState({ trashedPaths: new Map([[rec.path, outcome(4096, true)]]) }));
    expect(screen.getByText("Moved to Recycle Bin")).toBeTruthy();
    expect(
      screen.getByText("Empty the Recycle Bin to free 4.00 KB. Restore it from the Recycle Bin."),
    ).toBeTruthy();
  });
});
