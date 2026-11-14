import { describe, expect, it } from "vitest";
import type { Entry } from "../browse/entries";
import { useWindowsUserAgent } from "../shared/testPlatform";
import {
  type LayoutNode,
  layoutSunburst,
  MAX_RINGS,
  OTHER_THRESHOLD,
  percentOfParent,
  rootTotalBytes,
  segmentAriaLabel,
} from "./layout";

function dir(name: string, bytes: number | null = null, error: string | null = null): Entry {
  return { name, path: `/${name}`, isDir: true, bytes, modified: null, error };
}

function file(name: string, bytes: number | null = 1024): Entry {
  return { name, path: `/${name}`, isDir: false, bytes, modified: null, error: null };
}

function makeListings(
  root: Entry[],
  children?: Record<string, Entry[]>,
): ReadonlyMap<string, Entry[]> {
  const m = new Map<string, Entry[]>([["/root", root]]);
  if (children) {
    for (const [k, v] of Object.entries(children)) m.set(k, v);
  }
  return m;
}

describe("percentOfParent", () => {
  it("formats to one decimal place", () => {
    expect(percentOfParent(4.2e9, 12e9)).toBe("35.0%");
  });

  it("handles zero parent safely", () => {
    expect(percentOfParent(0, 0)).toBe("0.0%");
  });

  it("formats 100%", () => {
    expect(percentOfParent(5, 5)).toBe("100.0%");
  });
});

describe("rootTotalBytes", () => {
  it("sums entry bytes, treating null as 0", () => {
    const entries = [dir("a", 1000), dir("b", null), file("c", 500)];
    expect(rootTotalBytes("/root", entries)).toBe(1500);
  });
});

describe("segmentAriaLabel", () => {
  const base: LayoutNode = {
    id: "/a",
    name: "Downloads",
    path: "/a",
    depth: 0,
    startAngle: 0,
    endAngle: 1,
    innerRadius: 14,
    outerRadius: 25,
    bytes: 4.2e9,
    parentBytes: 12e9,
    kind: "folder",
    hue: 120,
    pending: false,
    clickable: true,
  };

  it("includes name, size and percentage", () => {
    const label = segmentAriaLabel(base);
    expect(label).toContain("Downloads");
    expect(label).toContain("35.0%");
  });

  it("shows measuring for pending nodes", () => {
    const label = segmentAriaLabel({ ...base, pending: true, bytes: null });
    expect(label).toContain("measuring");
  });
});

describe("layoutSunburst", () => {
  it("returns [] when root listing is absent", () => {
    const nodes = layoutSunburst({ rootPath: "/root", listings: new Map(), available: null });
    expect(nodes).toEqual([]);
  });

  it("returns [] for an empty root listing (EC-2)", () => {
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings([]),
      available: null,
    });
    expect(nodes).toEqual([]);
  });

  it("renders ring 0 arcs proportional to bytes", () => {
    const entries = [dir("a", 1000), dir("b", 3000)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const depth0 = nodes.filter((n) => n.depth === 0);
    expect(depth0).toHaveLength(2);
    const nodeA = depth0.find((n) => n.name === "a");
    const nodeB = depth0.find((n) => n.name === "b");
    expect(nodeA && nodeB).toBeTruthy();
    if (!nodeA || !nodeB) return;
    const sweepA = nodeA.endAngle - nodeA.startAngle;
    const sweepB = nodeB.endAngle - nodeB.startAngle;
    expect(sweepB / sweepA).toBeCloseTo(3);
  });

  it("groups tiny children into Other (AC-6 / FR-8)", () => {
    // 2 big dirs + 30 tiny dirs
    const entries: Entry[] = [
      dir("big1", 5e9),
      dir("big2", 5e9),
      ...Array.from({ length: 30 }, (_, i) => dir(`tiny${i}`, 1)),
    ];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const depth0 = nodes.filter((n) => n.depth === 0);
    const other = depth0.find((n) => n.kind === "other");
    expect(other).toBeTruthy();
    // big1, big2, Other — no individual tiny arcs
    expect(depth0.length).toBe(3);
    expect(depth0.filter((n) => n.kind === "folder")).toHaveLength(2);
  });

  it("does not merge pending dirs into Other (FR-10)", () => {
    const entries = [dir("big", 1e9), dir("pending", null), dir("pending2", null)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const depth0 = nodes.filter((n) => n.depth === 0);
    const other = depth0.find((n) => n.kind === "other");
    // pending dirs are tiny by weight (1 each) but must not go into Other
    expect(other).toBeUndefined();
    expect(depth0.filter((n) => n.pending)).toHaveLength(2);
  });

  it("emits a files aggregate for files-only folders (EC-3 / OQ-5)", () => {
    const entries = [file("photo.jpg", 1e6), file("video.mp4", 9e6)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    expect(nodes).toHaveLength(1);
    expect(nodes[0].kind).toBe("files");
    expect(nodes[0].name).toBe("Files");
    expect(nodes[0].clickable).toBe(false);
  });

  it("emits a files aggregate alongside dirs in mixed folders", () => {
    const entries = [dir("a", 1e9), file("photo.jpg", 5e8)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const depth0 = nodes.filter((n) => n.depth === 0);
    expect(depth0.find((n) => n.kind === "files")).toBeTruthy();
    expect(depth0.find((n) => n.kind === "folder")).toBeTruthy();
  });

  it("handles all-zero bytes with equal placeholder ring (EC-6)", () => {
    const entries = [dir("a", 0), dir("b", 0)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    expect(nodes).toHaveLength(2);
    const sweepA = nodes[0].endAngle - nodes[0].startAngle;
    const sweepB = nodes[1].endAngle - nodes[1].startAngle;
    expect(sweepA).toBeCloseTo(sweepB);
    expect(nodes.every((n) => n.pending)).toBe(true);
  });

  it("includes free space arc only at '/' with available !== null (FR-7 / EC-7)", () => {
    const listings = new Map([["/", [dir("a", 1e10)]]]);

    const withFree = layoutSunburst({ rootPath: "/", listings, available: 8e10 });
    expect(withFree.find((n) => n.kind === "free")).toBeTruthy();

    const withoutFree = layoutSunburst({ rootPath: "/", listings, available: null });
    expect(withoutFree.find((n) => n.kind === "free")).toBeUndefined();

    const nestedListings = new Map([["/home", [dir("a", 1e10)]]]);
    const nested = layoutSunburst({ rootPath: "/home", listings: nestedListings, available: 8e10 });
    expect(nested.find((n) => n.kind === "free")).toBeUndefined();
  });

  it("caps depth at MAX_RINGS - 1 (EC-9)", () => {
    const d0 = [dir("a", 1e10)];
    const d1 = [dir("b", 1e10)];
    const d2 = [dir("c", 1e10)];
    const d3 = [dir("d", 1e10)];
    const d4 = [dir("e", 1e10)];
    const listings = new Map([
      ["/root", d0],
      ["/a", d1],
      ["/b", d2],
      ["/c", d3],
      ["/d", d4],
    ]);
    const nodes = layoutSunburst({ rootPath: "/root", listings, available: null });
    const maxDepth = Math.max(...nodes.map((n) => n.depth));
    expect(maxDepth).toBe(MAX_RINGS - 1);
  });

  it("handles a 99% segment without overflow (EC-5)", () => {
    const entries = [dir("giant", 99), dir("tiny", 1)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const all = nodes.filter((n) => n.depth === 0);
    const totalSweep = all.reduce((s, n) => s + (n.endAngle - n.startAngle), 0);
    expect(totalSweep).toBeCloseTo(Math.PI * 2);
  });

  it("renders nested rings from child listings", () => {
    const childEntries = [dir("b1", 5e9), dir("b2", 5e9)];
    const listings = makeListings([dir("a", 1e10)], { "/a": childEntries });
    const nodes = layoutSunburst({ rootPath: "/root", listings, available: null });
    const depth1 = nodes.filter((n) => n.depth === 1);
    expect(depth1).toHaveLength(2);
  });

  it("groups 10k tiny children in under 50ms (NFR-2)", () => {
    const entries: Entry[] = Array.from({ length: 10000 }, (_, i) => ({
      name: `folder${i}`,
      path: `/root/folder${i}`,
      isDir: true,
      bytes: i < 5 ? 1e10 : 1,
      modified: null,
      error: null,
    }));
    const listings = new Map([["/root", entries]]);

    const t0 = performance.now();
    const nodes = layoutSunburst({ rootPath: "/root", listings, available: null });
    const elapsed = performance.now() - t0;

    expect(elapsed).toBeLessThan(50);
    const depth0 = nodes.filter((n) => n.depth === 0);
    // 5 big + 1 Other
    expect(depth0.length).toBeLessThanOrEqual(10);
    expect(depth0.find((n) => n.kind === "other")).toBeTruthy();
  });

  it("does not include free space in Other threshold computation", () => {
    // If free space is large, tiny dirs should still group under OTHER_THRESHOLD of parent total
    const entries = [dir("small", 1e6)];
    const listings = new Map([["/", entries]]);
    const nodes = layoutSunburst({ rootPath: "/", listings, available: 1e13 });
    // small is 1e6 / (1e6 + 1e13) ≈ 0.01% < 0.5% threshold → merged into Other
    const depth0 = nodes.filter((n) => n.depth === 0);
    expect(depth0.find((n) => n.kind === "other")).toBeTruthy();
    expect(depth0.find((n) => n.name === "small")).toBeUndefined();
  });

  it("parentBytes reflects actual bytes, not floor weight", () => {
    const entries = [dir("big", 1e9), dir("pending", null)];
    const nodes = layoutSunburst({
      rootPath: "/root",
      listings: makeListings(entries),
      available: null,
    });
    const bigNode = nodes.find((n) => n.name === "big");
    expect(bigNode?.parentBytes).toBe(1e9);
    expect(percentOfParent(bigNode?.bytes ?? 0, bigNode?.parentBytes ?? 0)).toBe("100.0%");
  });

  it("OTHER_THRESHOLD is 0.5%", () => {
    // Verify the constant
    expect(OTHER_THRESHOLD).toBe(0.005);
  });
});

describe("layoutSunburst free space on Windows", () => {
  useWindowsUserAgent();

  it.each([
    ["C:\\", true],
    ["D:\\", true],
    ["C:\\Users", false],
    ["", false],
  ])("free arc at %j is %s", (rootPath, expected) => {
    const listings = new Map([[rootPath, [dir("a", 1e10)]]]);
    const nodes = layoutSunburst({ rootPath, listings, available: 8e10 });
    expect(nodes.some((n) => n.kind === "free")).toBe(expected);
  });
});
