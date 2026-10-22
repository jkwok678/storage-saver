import { fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import type { LayoutNode } from "./layout";
import { SunburstChart } from "./SunburstChart";

function makeNode(overrides: Partial<LayoutNode> = {}): LayoutNode {
  return {
    id: "/folder",
    name: "folder",
    path: "/folder",
    depth: 0,
    startAngle: 0,
    endAngle: Math.PI,
    innerRadius: 14,
    outerRadius: 25,
    bytes: 4.2e9,
    parentBytes: 12e9,
    kind: "folder",
    hue: 120,
    pending: false,
    clickable: true,
    ...overrides,
  };
}

const defaultProps = {
  rootPath: "/",
  nodes: [] as LayoutNode[],
  totalBytes: 0,
  scanning: false,
  onZoom: vi.fn(),
  onReveal: vi.fn(),
};

function renderChart(overrides: Partial<ComponentProps<typeof SunburstChart>> = {}) {
  return render(<SunburstChart {...defaultProps} {...overrides} />);
}

function arc(name: string, role: "button" | "img" = "button") {
  return screen.getByRole(role, { name: new RegExp(name) });
}

describe("SunburstChart", () => {
  it("renders an accessible figure with aria-label", () => {
    renderChart({ rootPath: "/Users", totalBytes: 1e9 });
    const fig = screen.getByRole("figure");
    expect(fig.getAttribute("aria-label")).toContain("Users");
  });

  it("renders a path element for each node", () => {
    const nodes = [
      makeNode(),
      makeNode({
        id: "/b",
        path: "/b",
        name: "other-folder",
        startAngle: Math.PI,
        endAngle: Math.PI * 2,
      }),
    ];
    renderChart({ nodes, totalBytes: 8.4e9 });
    expect(screen.getAllByRole("button").length).toBe(2);
  });

  it("shows tooltip with name, size and percentage on hover", () => {
    renderChart({ nodes: [makeNode({ bytes: 4.2e9, parentBytes: 12e9 })], totalBytes: 4.2e9 });
    fireEvent.mouseEnter(arc("folder"), { clientX: 200, clientY: 150 });
    expect(document.querySelector(".tooltip-size")?.textContent).toBe("3.91 GB");
    expect(screen.getByText("35.0%")).toBeTruthy();
  });

  it("hides tooltip on mouse leave", () => {
    renderChart({ nodes: [makeNode()], totalBytes: 1e9 });
    const path = arc("folder");
    fireEvent.mouseEnter(path, { clientX: 100, clientY: 100 });
    expect(document.querySelector(".sunburst-tooltip")).toBeTruthy();
    fireEvent.mouseLeave(path);
    expect(document.querySelector(".sunburst-tooltip")).toBeNull();
  });

  it("shows Measuring… in tooltip for pending nodes", () => {
    renderChart({ nodes: [makeNode({ pending: true, bytes: null })], totalBytes: 0 });
    fireEvent.mouseEnter(arc("folder"), { clientX: 0, clientY: 0 });
    expect(screen.getAllByText("Measuring…").length).toBeGreaterThan(0);
  });

  it("calls onZoom when clicking a clickable folder node", () => {
    const onZoom = vi.fn();
    renderChart({ nodes: [makeNode()], onZoom, totalBytes: 1e9 });
    fireEvent.click(arc("folder"));
    expect(onZoom).toHaveBeenCalledWith("/folder");
  });

  it("does not call onZoom for non-clickable nodes", () => {
    const onZoom = vi.fn();
    renderChart({
      nodes: [makeNode({ kind: "other", clickable: false, path: null, name: "Other" })],
      onZoom,
      totalBytes: 1e9,
    });
    fireEvent.click(arc("Other", "img"));
    expect(onZoom).not.toHaveBeenCalled();
  });

  it("calls onZoom on Enter key for focusable folder", () => {
    const onZoom = vi.fn();
    renderChart({ nodes: [makeNode()], onZoom, totalBytes: 1e9 });
    fireEvent.keyDown(arc("folder"), { key: "Enter" });
    expect(onZoom).toHaveBeenCalledWith("/folder");
  });

  it("calls onZoom on Space key for focusable folder", () => {
    const onZoom = vi.fn();
    renderChart({ nodes: [makeNode()], onZoom, totalBytes: 1e9 });
    fireEvent.keyDown(arc("folder"), { key: " " });
    expect(onZoom).toHaveBeenCalledWith("/folder");
  });

  it("calls onReveal on context menu for folder node", () => {
    const onReveal = vi.fn();
    renderChart({ nodes: [makeNode()], onReveal, totalBytes: 1e9 });
    fireEvent.contextMenu(arc("folder"));
    expect(onReveal).toHaveBeenCalledWith("/folder");
  });

  it("does not call onReveal for non-folder nodes", () => {
    const onReveal = vi.fn();
    renderChart({
      nodes: [makeNode({ kind: "other", path: null, clickable: false, name: "Other" })],
      onReveal,
      totalBytes: 1e9,
    });
    fireEvent.contextMenu(arc("Other", "img"));
    expect(onReveal).not.toHaveBeenCalled();
  });

  it("shows Measuring… in centre while scanning with zero bytes", () => {
    renderChart({ nodes: [], totalBytes: 0, scanning: true, rootPath: "/Users/alice" });
    expect(screen.getAllByText("Measuring…").length).toBeGreaterThan(0);
  });

  it("shows formatted total bytes in centre when not scanning", () => {
    renderChart({ nodes: [], totalBytes: 5.0 * 1024 ** 3, scanning: false, rootPath: "/" });
    expect(screen.getByText("5.00 GB")).toBeTruthy();
  });

  it("sets tabIndex=0 and role=button only on clickable nodes", () => {
    renderChart({
      nodes: [
        makeNode({ id: "/a", path: "/a", name: "Projects", clickable: true }),
        makeNode({
          id: "/b::__other__",
          path: null,
          name: "Other",
          kind: "other",
          clickable: false,
          hue: null,
        }),
      ],
      totalBytes: 1e9,
    });
    expect(arc("Projects").getAttribute("tabindex")).toBe("0");
    expect(arc("Other", "img").getAttribute("tabindex")).toBeNull();
  });
});
