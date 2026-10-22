import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiProviders } from "./aiProviders";
import SettingsView from "./SettingsView";

vi.mock("./format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./format")>()),
  runningInApp: () => true,
}));

function providers(overrides: Partial<AiProviders> = {}): AiProviders {
  return {
    providers: [
      { id: "claude", name: "Claude Code", installed: true, connected: true, account: "me" },
      { id: "cursor", name: "Cursor", installed: true, connected: false },
    ],
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
});

describe("SettingsView", () => {
  it("signs out, connects and refreshes", () => {
    const ai = providers();
    render(<SettingsView ai={ai} />);
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    expect(ai.disconnect).toHaveBeenCalledWith("claude");
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(ai.connect).toHaveBeenCalledWith("cursor");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(ai.refresh).toHaveBeenCalled();
  });

  it("shows progress and locks the buttons while busy", () => {
    const { rerender } = render(<SettingsView ai={providers({ busy: "claude" })} />);
    const signingOut = screen.getByRole("button", { name: "Signing out…" }) as HTMLButtonElement;
    expect(signingOut.disabled).toBe(true);
    rerender(<SettingsView ai={providers({ busy: "cursor", error: "Login cancelled" })} />);
    expect(screen.getByRole("button", { name: "Connecting…" })).toBeTruthy();
    expect(screen.getByText("Login cancelled")).toBeTruthy();
  });

  it("offers no account buttons while checking or without the CLI", () => {
    const { rerender } = render(
      <SettingsView ai={providers({ providers: null, checking: true })} />,
    );
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
    rerender(
      <SettingsView
        ai={providers({
          providers: [{ id: "cursor", name: "Cursor", installed: false, connected: false }],
        })}
      />,
    );
    expect(screen.getByText("Cursor CLI not found")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Connect" })).toHaveLength(1);
  });
});
