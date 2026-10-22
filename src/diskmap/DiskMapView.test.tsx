import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import DiskMapView from "./DiskMapView";
import type { SunburstScan } from "./useSunburstScan";
import { useSunburstScan } from "./useSunburstScan";

// ---- module mocks ----

const runningInApp = vi.hoisted(() => vi.fn(() => true));
vi.mock("../shared/format", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../shared/format")>()),
  runningInApp,
}));

vi.mock("./useSunburstScan", () => ({ useSunburstScan: vi.fn() }));
vi.mock("./SunburstChart", () => ({ SunburstChart: () => <div data-testid="chart" /> }));

const revealItemInDir = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir }));

// ---- helpers ----

const mockRefresh = vi.fn();

function scanState(overrides: Partial<SunburstScan> = {}) {
  vi.mocked(useSunburstScan).mockReturnValue({
    listings: new Map(),
    rootEntries: [],
    scanning: false,
    rootScanning: false,
    scanError: null,
    refresh: mockRefresh,
    ...overrides,
  });
}

describe("DiskMapView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    runningInApp.mockReturnValue(true);
    revealItemInDir.mockResolvedValue(undefined);
    scanState();
  });

  it("shows dev-mode message outside Tauri (EC-8)", () => {
    runningInApp.mockReturnValue(false);
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    expect(screen.getAllByText("Open Storage Saver to read this disk.").length).toBeGreaterThan(0);
    expect(screen.queryByTestId("chart")).toBeNull();
  });

  it("shows 'This folder is empty.' when root has no entries (EC-2)", () => {
    scanState({ rootEntries: [], scanning: false });
    render(<DiskMapView initialPath="/empty" onPathChange={vi.fn()} available={null} />);
    expect(screen.getByText("This folder is empty.")).toBeTruthy();
  });

  it("shows error from root scan (EC-1)", () => {
    scanState({ scanError: "Permission denied" });
    render(<DiskMapView initialPath="/private" onPathChange={vi.fn()} available={null} />);
    // Error appears in both the status line and the chart area
    expect(screen.getAllByText("Permission denied").length).toBeGreaterThanOrEqual(1);
    // The panel-level error has class "error"
    expect(screen.getByText("Permission denied", { selector: ".error" })).toBeTruthy();
  });

  it("renders the chart when entries exist", () => {
    scanState({
      rootEntries: [
        { name: "a", path: "/a", isDir: true, bytes: 1e9, modified: null, error: null },
      ],
    });
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    expect(screen.getByTestId("chart")).toBeTruthy();
  });

  it("shows scan status while root scanning", () => {
    scanState({ rootScanning: true, scanning: true, rootEntries: [] });
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    expect(screen.getByText("Reading this folder…")).toBeTruthy();
  });

  it("shows nested-folder status while nested jobs run", () => {
    scanState({
      scanning: true,
      rootScanning: false,
      rootEntries: [
        { name: "a", path: "/a", isDir: true, bytes: 1e9, modified: null, error: null },
      ],
    });
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    expect(screen.getByText("Reading nested folders…")).toBeTruthy();
  });

  it("calls hook refresh when Refresh button clicked (FR-11)", () => {
    scanState({
      rootEntries: [{ name: "a", path: "/a", isDir: true, bytes: 1, modified: null, error: null }],
    });
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(mockRefresh).toHaveBeenCalled();
  });

  it("disables Refresh while scanning", () => {
    scanState({ scanning: true, rootScanning: true, rootEntries: [] });
    render(<DiskMapView initialPath="/" onPathChange={vi.fn()} available={null} />);
    expect(screen.getByRole("button", { name: "Refresh" }).hasAttribute("disabled")).toBe(true);
  });

  it("navigates via breadcrumb and calls onPathChange (FR-6)", () => {
    const onPathChange = vi.fn();
    scanState();
    render(<DiskMapView initialPath="/Users/alice" onPathChange={onPathChange} available={null} />);
    fireEvent.click(screen.getByRole("button", { name: "This Mac" }));
    expect(onPathChange).toHaveBeenCalledWith("/");
  });

  it("shows breadcrumb for the current path as static text", () => {
    scanState();
    render(<DiskMapView initialPath="/Users/alice" onPathChange={vi.fn()} available={null} />);
    // "alice" is the current crumb — rendered as <span> not <button>
    expect(screen.queryByRole("button", { name: "alice" })).toBeNull();
    expect(screen.getByText("alice")).toBeTruthy();
  });

  it("navigates to parent when trashed item is within current path", () => {
    const onPathChange = vi.fn();
    scanState();
    const { rerender } = render(
      <DiskMapView initialPath="/Users/alice" onPathChange={onPathChange} available={null} />,
    );
    rerender(
      <DiskMapView
        initialPath="/Users/alice"
        onPathChange={onPathChange}
        available={null}
        trashed={{ path: "/Users/alice", seq: 1 }}
      />,
    );
    expect(onPathChange).toHaveBeenCalledWith("/Users");
  });

  it("re-scans without navigating when a child item is trashed", () => {
    const onPathChange = vi.fn();
    scanState();
    const { rerender } = render(
      <DiskMapView initialPath="/Users/alice" onPathChange={onPathChange} available={null} />,
    );
    rerender(
      <DiskMapView
        initialPath="/Users/alice"
        onPathChange={onPathChange}
        available={null}
        trashed={{ path: "/Users/alice/file.txt", seq: 1 }}
      />,
    );
    // No navigation — onPathChange not called, but useSunburstScan gets a new request
    expect(onPathChange).not.toHaveBeenCalled();
    const lastReq = vi.mocked(useSunburstScan).mock.lastCall?.[0];
    expect(lastReq?.path).toBe("/Users/alice");
  });
});
