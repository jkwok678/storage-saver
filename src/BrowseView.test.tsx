import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import BrowseView from "./BrowseView";
import type { Entry } from "./entries";
import { useDirectoryScan } from "./useDirectoryScan";

vi.mock("./useDirectoryScan", () => ({ useDirectoryScan: vi.fn() }));

function entry(name: string, overrides: Partial<Entry> = {}): Entry {
  return {
    name,
    path: `/Users/${name}`,
    isDir: false,
    bytes: 1024,
    modified: null,
    error: null,
    ...overrides,
  };
}

const entries = [
  entry("notes.txt", { bytes: 2048 }),
  entry("Projects", { isDir: true }),
  entry("Locked", { isDir: true, error: "Permission denied" }),
];

function scanState(overrides: Partial<ReturnType<typeof useDirectoryScan>> = {}) {
  vi.mocked(useDirectoryScan).mockReturnValue({
    entries,
    scanning: false,
    scanError: null,
    ...overrides,
  });
}

function renderView(onPathChange = vi.fn()) {
  render(<BrowseView initialPath="/Users" onPathChange={onPathChange} available={5 * 1024} />);
  return onPathChange;
}

const rowNames = () =>
  screen
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getByRole("rowheader").textContent);

const lastRequest = () => vi.mocked(useDirectoryScan).mock.lastCall?.[0];

describe("BrowseView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    scanState();
  });

  it("lists entries with folders first and shows totals", () => {
    renderView();
    expect(rowNames()).toEqual(["Locked", "Projects›", "notes.txt"]);
    expect(screen.getAllByText("3 items")).toHaveLength(2);
    expect(screen.getByText("5.00 KB available")).toBeTruthy();
    expect(screen.getByText("Permission denied")).toBeTruthy();
  });

  it("toggles sort direction from the column headers", () => {
    renderView();
    const name = screen.getByRole("columnheader", { name: /Name/ });
    expect(name.getAttribute("aria-sort")).toBe("ascending");
    fireEvent.click(within(name).getByRole("button"));
    expect(name.getAttribute("aria-sort")).toBe("descending");
    expect(rowNames()).toEqual(["Projects›", "Locked", "notes.txt"]);

    const size = screen.getByRole("columnheader", { name: /Size/ });
    fireEvent.click(within(size).getByRole("button"));
    expect(size.getAttribute("aria-sort")).toBe("descending");
    expect(name.getAttribute("aria-sort")).toBe("none");
  });

  it("selects a row and opens folders on double-click or Enter", () => {
    const onPathChange = renderView();
    const projects = screen.getByText("Projects").closest("tr") as HTMLElement;
    fireEvent.click(projects);
    expect(projects.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText(/Projects selected/)).toBeTruthy();

    fireEvent.doubleClick(projects);
    expect(onPathChange).toHaveBeenLastCalledWith("/Users/Projects");
    expect(lastRequest()).toEqual({ path: "/Users/Projects", refresh: false });

    fireEvent.keyDown(screen.getByText("Locked").closest("tr") as HTMLElement, { key: "Enter" });
    fireEvent.doubleClick(screen.getByText("notes.txt").closest("tr") as HTMLElement);
    expect(onPathChange).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(projects, { key: "Enter" });
    expect(onPathChange).toHaveBeenCalledTimes(2);
  });

  it("navigates via breadcrumbs and refreshes the current folder", () => {
    const onPathChange = renderView();
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(lastRequest()).toEqual({ path: "/Users", refresh: true });
    fireEvent.click(screen.getByRole("button", { name: "This Mac" }));
    expect(onPathChange).toHaveBeenCalledWith("/");
  });

  it("shows scan progress, errors and empty folders", () => {
    scanState({ entries: [], scanning: true });
    renderView();
    expect(screen.getByText("Reading this folder…")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Refresh" }).hasAttribute("disabled")).toBe(true);
    expect(screen.queryByText("This folder is empty.")).toBeNull();
  });

  it("says when a folder is empty", () => {
    scanState({ entries: [] });
    renderView();
    expect(screen.getByText("This folder is empty.")).toBeTruthy();
  });
});
