import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "./aiTypes";
import { ChatMessageRow } from "./ChatMessageRow";
import type { TrashState } from "./useTrash";

const trash: TrashState = {
  confirmingTrash: null,
  setConfirmingTrash: vi.fn(),
  trashingPath: null,
  trashedPaths: new Map(),
  trashError: null,
  clearTrashError: vi.fn(),
  reveal: vi.fn(),
  moveToTrash: vi.fn(),
};

function renderRow(msg: Partial<ChatMessage>) {
  render(
    <ChatMessageRow
      msg={{ id: "a-1", role: "assistant", content: "", ...msg }}
      fallbackProviderName="Cursor"
      trash={trash}
    />,
  );
}

describe("ChatMessageRow", () => {
  it("labels a reply with who answered and the model", () => {
    renderRow({ content: "**Hi**", providerName: "Claude Code", modelLabel: "Opus" });
    expect(screen.getByText("Claude Code · Opus")).toBeTruthy();
    expect(screen.getByText("Hi").tagName).toBe("STRONG");
  });

  it("falls back to the current provider and leaves user messages unlabelled", () => {
    renderRow({});
    expect(screen.getByText("Cursor")).toBeTruthy();
    renderRow({ role: "user", content: "Question" });
    expect(screen.getAllByText(/Cursor/)).toHaveLength(1);
  });

  it("notes a stopped reply", () => {
    renderRow({ outcome: "stopped" });
    expect(screen.getByText("Stopped before answering.")).toBeTruthy();
    renderRow({ outcome: "stopped", content: "Partial" });
    expect(screen.getByText("Stopped.")).toBeTruthy();
  });

  it("lists recommendations", () => {
    renderRow({
      recommendations: [
        {
          path: "/a",
          name: "a",
          isDir: false,
          bytes: 1,
          safety: "safe",
          reason: "",
          exists: true,
        },
      ],
    });
    expect(screen.getByText("Recommended for cleanup (1)")).toBeTruthy();
  });
});
