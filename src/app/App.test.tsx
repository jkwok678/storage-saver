import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWindowsUserAgent } from "../shared/testPlatform";
import App from "./App";
import { useDiskSummary } from "./useDiskSummary";

vi.mock("./useDiskSummary", () => ({ useDiskSummary: vi.fn() }));
vi.mock("../browse/BrowseView", () => ({
  default: ({ available }: { available: number | null }) => <p>browse {String(available)}</p>,
}));
vi.mock("../largest/LargestFilesView", () => ({ default: () => <p>largest</p> }));
vi.mock("../settings/SettingsView", () => ({ default: () => <p>settings</p> }));
vi.mock("../diskmap/DiskMapView", () => ({ default: () => <p>diskmap</p> }));
type PanelProps = {
  hidden?: boolean;
  onClose: () => void;
  onOpenSettings: () => void;
  onItemTrashed?: (path: string, freedBytes: number) => void;
};
vi.mock("../ai/AiAdvisorPanel", () => ({
  AiAdvisorPanel: ({ hidden, onClose, onOpenSettings, onItemTrashed }: PanelProps) => (
    <aside hidden={hidden}>
      <button type="button" onClick={onClose}>
        close panel
      </button>
      <button type="button" onClick={onOpenSettings}>
        open settings
      </button>
      <button type="button" onClick={() => onItemTrashed?.("/a", 0)}>
        trash item
      </button>
    </aside>
  ),
}));

const GB = 1024 ** 3;
const refresh = vi.fn();

describe("App", () => {
  beforeEach(() => {
    vi.mocked(useDiskSummary).mockReturnValue({ summary: null, error: null, refresh });
  });

  it("shows disk capacity once the summary loads", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 100 * GB, used: 25 * GB, available: 75 * GB },
      error: null,
      refresh,
    });
    render(<App />);
    const meter = screen.getByRole("meter", { name: "Boot disk usage" });
    expect(meter.getAttribute("value")).toBe(String(25 * GB));
    expect(screen.getByText(/25% full · 25.0 GB used/)).toBeTruthy();
    expect(screen.getByText(`browse ${75 * GB}`)).toBeTruthy();
  });

  it("shows loading, then errors, in place of the meter", () => {
    const { rerender } = render(<App />);
    expect(screen.getByText("Reading disk…")).toBeTruthy();
    expect(screen.getByText("browse null")).toBeTruthy();
    vi.mocked(useDiskSummary).mockReturnValue({ summary: null, error: "No access", refresh });
    rerender(<App />);
    expect(screen.getByText("No access").className).toBe("error");
  });

  it("treats an empty disk as 0% full", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 0, used: 0, available: 0 },
      error: null,
      refresh,
    });
    render(<App />);
    expect(screen.getByText(/0% full/)).toBeTruthy();
  });

  it("switches between views", () => {
    render(<App />);
    const largest = screen.getByRole("button", { name: "Largest Files" });
    fireEvent.click(largest);
    expect(largest.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("largest")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    expect(screen.getByText(/^browse/)).toBeTruthy();
  });

  it("shows the Disk Map tab and renders DiskMapView when clicked", () => {
    render(<App />);
    const diskMap = screen.getByRole("button", { name: "Disk Map" });
    expect(diskMap).toBeTruthy();
    fireEvent.click(diskMap);
    expect(diskMap.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("diskmap")).toBeTruthy();
  });

  it("preserves browsePath when switching to Disk Map and back", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 100 * GB, used: 25 * GB, available: 75 * GB },
      error: null,
      refresh,
    });
    render(<App />);
    // Default view is Browse — shows available as browse prop
    expect(screen.getByText(`browse ${75 * GB}`)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Disk Map" }));
    expect(screen.getByText("diskmap")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Browse" }));
    expect(screen.getByText(`browse ${75 * GB}`)).toBeTruthy();
  });

  it("opens Settings from the gear, the panel and ⌘,", () => {
    render(<App />);
    const gear = screen.getByRole("button", { name: "Settings" });
    fireEvent.click(gear);
    expect(screen.getByText("settings")).toBeTruthy();
    fireEvent.click(gear);
    expect(screen.getByText(/^browse/)).toBeTruthy();
    fireEvent.keyDown(window, { key: ",", metaKey: true });
    expect(screen.getByText("settings")).toBeTruthy();
  });

  it("keeps the AI panel mounted once opened and refreshes after a trash", () => {
    render(<App />);
    expect(screen.queryByRole("button", { name: "close panel" })).toBeNull();
    const toggle = screen.getByRole("button", { name: "AI Advisor" });
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "trash item" }));
    expect(refresh).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "open settings" }));
    expect(screen.getByText("settings")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "close panel" }));
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByRole("button", { name: "close panel", hidden: true })).toBeTruthy();
  });
});

describe("App on Windows", () => {
  useWindowsUserAgent();

  it("reads the PC disk, labels the meter and opens Settings with Ctrl+,", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 100 * GB, used: 25 * GB, available: 75 * GB },
      error: null,
      refresh,
    });
    render(<App />);
    expect(useDiskSummary).toHaveBeenLastCalledWith("");
    expect(screen.getByRole("meter", { name: "This PC disk usage" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Settings" }).title).toBe("Settings (Ctrl+,)");
    fireEvent.keyDown(window, { key: ",", metaKey: true });
    expect(screen.queryByText("settings")).toBeNull();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(screen.getByText("settings")).toBeTruthy();
  });
});
