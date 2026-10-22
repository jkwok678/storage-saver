import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { useDiskSummary } from "./useDiskSummary";

vi.mock("./useDiskSummary", () => ({ useDiskSummary: vi.fn() }));
vi.mock("./BrowseView", () => ({
  default: ({ available }: { available: number | null }) => <p>browse {String(available)}</p>,
}));
vi.mock("./LargestFilesView", () => ({ default: () => <p>largest</p> }));

const GB = 1024 ** 3;

describe("App", () => {
  beforeEach(() => {
    vi.mocked(useDiskSummary).mockReturnValue({ summary: null, error: null });
  });

  it("shows disk capacity once the summary loads", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 100 * GB, used: 25 * GB, available: 75 * GB },
      error: null,
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
    vi.mocked(useDiskSummary).mockReturnValue({ summary: null, error: "No access" });
    rerender(<App />);
    expect(screen.getByText("No access").className).toBe("error");
  });

  it("treats an empty disk as 0% full", () => {
    vi.mocked(useDiskSummary).mockReturnValue({
      summary: { total: 0, used: 0, available: 0 },
      error: null,
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
});
