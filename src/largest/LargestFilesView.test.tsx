import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWindowsUserAgent } from "../shared/testPlatform";
import LargestFilesView from "./LargestFilesView";
import { useLargestScan } from "./useLargestScan";

vi.mock("./useLargestScan", () => ({ useLargestScan: vi.fn() }));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp: () => true,
}));
const revealItemInDir = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir }));

const stop = vi.fn();
const rescan = vi.fn();
const forget = vi.fn();

function scanState(overrides: Partial<ReturnType<typeof useLargestScan>> = {}) {
  vi.mocked(useLargestScan).mockReturnValue({
    files: [],
    scanned: 0,
    scanning: false,
    error: null,
    stop,
    rescan,
    forget,
    ...overrides,
  });
}

const files = [
  { name: "movie.mov", path: "/Users/me/movie.mov", bytes: 4096 },
  { name: "disk.dmg", path: "/disk.dmg", bytes: 1024 },
];

describe("LargestFilesView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    revealItemInDir.mockResolvedValue(undefined);
  });

  it("shows progress and stops while scanning", () => {
    scanState({ scanning: true, scanned: 1500 });
    render(<LargestFilesView />);
    expect(screen.getByText(/1,500 files read/)).toBeTruthy();
    expect(screen.getByText("Looking for the biggest files…")).toBeTruthy();
    expect(screen.getByText("Largest files found so far")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(stop).toHaveBeenCalled();
  });

  it("lists ranked files with their parent folder", () => {
    scanState({ files, scanned: 10 });
    render(<LargestFilesView />);
    expect(screen.getByText("Top 2 of 10 files")).toBeTruthy();
    expect(screen.getByText("/Users/me")).toBeTruthy();
    expect(screen.getByText("4.00 KB")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Scan again" }));
    expect(rescan).toHaveBeenCalled();
  });

  it("reveals a file on double-click or Enter", () => {
    scanState({ files });
    render(<LargestFilesView />);
    const row = screen.getByText("disk.dmg").closest("button") as HTMLButtonElement;
    fireEvent.doubleClick(row);
    fireEvent.keyDown(row, { key: "Enter" });
    fireEvent.keyDown(row, { key: "a" });
    expect(revealItemInDir).toHaveBeenCalledTimes(2);
    expect(revealItemInDir).toHaveBeenCalledWith("/disk.dmg");
  });

  it("shows a reveal failure until the next scan", async () => {
    revealItemInDir.mockRejectedValue("No such file");
    scanState({ files });
    render(<LargestFilesView />);
    fireEvent.doubleClick(screen.getByText("disk.dmg"));
    expect(await screen.findByText("No such file")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Scan again" }));
    expect(screen.queryByText("No such file")).toBeNull();
  });

  it("shows scan errors and an empty result", () => {
    scanState({ error: "Full Disk Access required" });
    const { rerender } = render(<LargestFilesView />);
    expect(screen.getAllByText("Full Disk Access required")).toHaveLength(2);
    scanState();
    rerender(<LargestFilesView />);
    expect(screen.getByText("No files found.")).toBeTruthy();
  });
});

describe("LargestFilesView on Windows", () => {
  useWindowsUserAgent();

  it("names this PC and File Explorer", () => {
    scanState({ files, scanning: true, scanned: 3 });
    render(<LargestFilesView />);
    expect(screen.getByText(/Scanning this PC/)).toBeTruthy();
    expect(screen.getByText("Double-click a file to show it in File Explorer")).toBeTruthy();
  });
});
