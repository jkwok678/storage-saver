import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AiAdvisorPanel } from "./AiAdvisorPanel";
import type { AiProviderStatus, AiProviders } from "./aiProviders";
import { type AiChat, useAiChat } from "./useAiChat";
import { useModelPicker } from "./useModelPicker";

vi.mock("./useAiChat", () => ({ useAiChat: vi.fn() }));
vi.mock("./useModelPicker", () => ({ useModelPicker: vi.fn() }));
const app = vi.hoisted(() => ({ running: true }));
vi.mock("./format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./format")>()),
  runningInApp: () => app.running,
}));

const claude: AiProviderStatus = {
  id: "claude",
  name: "Claude Code",
  installed: true,
  connected: false,
};
const cursor: AiProviderStatus = {
  id: "cursor",
  name: "Cursor",
  installed: true,
  connected: true,
  account: "me@example.com",
};

function providers(overrides: Partial<AiProviders> = {}): AiProviders {
  return {
    providers: [claude, cursor],
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

function chatState(overrides: Partial<AiChat> = {}): AiChat {
  const chat: AiChat = {
    messages: [],
    input: "",
    setInput: vi.fn(),
    querying: false,
    statusMessage: null,
    queryError: null,
    sendMessage: vi.fn(),
    stop: vi.fn(),
    clear: vi.fn(),
    ...overrides,
  };
  vi.mocked(useAiChat).mockReturnValue(chat);
  return chat;
}

const onOpenSettings = vi.fn();

function renderPanel(ai = providers()) {
  return render(
    <AiAdvisorPanel ai={ai} currentPath="/" onClose={vi.fn()} onOpenSettings={onOpenSettings} />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  app.running = true;
  vi.mocked(useModelPicker).mockReturnValue({
    choices: undefined,
    pick: { model: null, effort: null },
    modelChoice: undefined,
    effortChoice: null,
    catalogError: undefined,
    updatePick: vi.fn(),
  });
});

const tab = (name: string) => screen.getByRole("tab", { name });

describe("AiAdvisorPanel", () => {
  it("starts on a signed-in provider and lets the user switch", () => {
    chatState();
    renderPanel();
    expect(tab("Cursor").getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("me@example.com")).toBeTruthy();
    fireEvent.click(tab("Claude Code"));
    expect(tab("Claude Code").getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("Not signed in")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Connect in Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });

  it("keeps the user's provider when statuses refresh", () => {
    chatState();
    const { rerender } = renderPanel(providers({ providers: null, checking: true }));
    expect(screen.getByText("Checking connection…")).toBeTruthy();
    rerender(
      <AiAdvisorPanel
        ai={providers()}
        currentPath="/"
        onClose={vi.fn()}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(tab("Cursor").getAttribute("aria-selected")).toBe("true");
    fireEvent.click(tab("Claude Code"));
    rerender(
      <AiAdvisorPanel
        ai={providers({ providers: [cursor] })}
        currentPath="/"
        onClose={vi.fn()}
        onOpenSettings={onOpenSettings}
      />,
    );
    expect(tab("Claude Code").getAttribute("aria-selected")).toBe("true");
  });

  it("asks a suggested question", () => {
    const chat = chatState();
    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: /safely delete right now/ }));
    expect(chat.sendMessage).toHaveBeenCalledWith(
      "What can I safely delete right now to free up space?",
    );
  });

  it("sends with Enter but not Shift-Enter", () => {
    const chat = chatState({ input: "Why is it full?" });
    renderPanel();
    const box = screen.getByRole("textbox");
    fireEvent.change(box, { target: { value: "x" } });
    expect(chat.setInput).toHaveBeenCalledWith("x");
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    expect(chat.sendMessage).not.toHaveBeenCalled();
    fireEvent.keyDown(box, { key: "Enter" });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(chat.sendMessage).toHaveBeenCalledTimes(2);
  });

  it("shows a running reply with ways to stop it", () => {
    const chat = chatState({
      querying: true,
      statusMessage: "Reading folders…",
      messages: [{ id: "u-1", role: "user", content: "Hi" }],
    });
    renderPanel();
    expect(screen.getByText("Reading folders…")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop responding" }));
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(chat.stop).toHaveBeenCalledTimes(2);
  });

  it("clears the conversation and links a failed query to Settings", () => {
    const chat = chatState({
      queryError: "Not logged in",
      messages: [{ id: "u-1", role: "user", content: "Hi" }],
    });
    renderPanel(providers({ providers: [claude] }));
    fireEvent.click(screen.getByRole("button", { name: "Clear conversation" }));
    expect(chat.clear).toHaveBeenCalled();
    expect(screen.getByText("Not logged in")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Sign in to Claude Code in Settings" }));
    expect(onOpenSettings).toHaveBeenCalled();
  });

  it("explains it needs the app", () => {
    app.running = false;
    chatState();
    renderPanel();
    expect(screen.getByText("Open Storage Saver to use the AI Advisor")).toBeTruthy();
  });
});
