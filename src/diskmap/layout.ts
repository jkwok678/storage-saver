import type { Entry } from "../browse/entries";
import { formatBytes } from "../shared/format";
import { platform } from "../shared/platform";
import { hueFromName } from "./color";

export const MAX_RINGS = 4;
export const OTHER_THRESHOLD = 0.005;
export const MIN_EXPAND_TURN = 2 / 360;
export const VIEWBOX = 100;
export const CENTER = 50;
const OUTER_RADIUS = 48;
const HOLE_RATIO = 0.28;

export type ArcKind = "folder" | "files" | "other" | "free";

export type LayoutNode = {
  id: string;
  name: string;
  path: string | null;
  depth: number;
  startAngle: number;
  endAngle: number;
  innerRadius: number;
  outerRadius: number;
  bytes: number | null;
  parentBytes: number;
  kind: ArcKind;
  hue: number | null;
  pending: boolean;
  clickable: boolean;
};

export type LayoutInput = {
  rootPath: string;
  listings: ReadonlyMap<string, Entry[]>;
  available: number | null;
};

type GroupItem = {
  id: string;
  name: string;
  path: string | null;
  kind: ArcKind;
  bytes: number | null;
  weight: number;
  pending: boolean;
  isError: boolean;
};

const collator = new Intl.Collator(undefined, { sensitivity: "base" });

function sumBytes(entries: Entry[]): number {
  return entries.reduce((s, e) => s + (e.bytes ?? 0), 0);
}

function dirItem(d: Entry): GroupItem {
  const isPending = d.bytes === null && d.error === null;
  return {
    id: d.path,
    name: d.name,
    path: d.path,
    kind: "folder",
    bytes: d.bytes,
    weight: isPending ? 1 : (d.bytes ?? 0),
    pending: isPending,
    isError: d.error !== null,
  };
}

function syntheticItem(
  listingPath: string,
  kind: Exclude<ArcKind, "folder">,
  name: string,
  bytes: number,
): GroupItem {
  return {
    id: `${listingPath}::__${kind}__`,
    name,
    path: null,
    kind,
    bytes,
    weight: bytes,
    pending: false,
    isError: false,
  };
}

function isSmallDir(d: GroupItem, knownTotal: number): boolean {
  return !d.pending && !d.isError && d.weight / knownTotal < OTHER_THRESHOLD;
}

function groupListing(
  entries: Entry[],
  listingPath: string,
  includeFree: boolean,
  available: number | null,
): { items: GroupItem[]; bytesTotal: number } {
  const dirs = entries.filter((e) => e.isDir);
  const files = entries.filter((e) => !e.isDir);

  const fileBytes = sumBytes(files);
  const hasFilesAgg = fileBytes > 0 || (files.length > 0 && dirs.length === 0);
  const dirItems = dirs.map(dirItem);

  const freeBytes = includeFree ? available : null;
  const freeWeight = freeBytes ?? 0;
  const knownTotal = dirItems.reduce((s, d) => s + d.weight, 0) + fileBytes + freeWeight;

  // All items with actual bytes (no floor), for parentBytes display
  const bytesTotal = sumBytes(dirs) + fileBytes + freeWeight;

  // Handle all-zero: equal sweeps, pending style
  if (knownTotal === 0) {
    const zeroItems = hasFilesAgg
      ? [...dirItems, syntheticItem(listingPath, "files", "Files", 0)]
      : dirItems;
    return {
      items: zeroItems.map((item) => ({ ...item, weight: 1, pending: true })),
      bytesTotal: 0,
    };
  }

  // Other grouping: non-pending, non-error dirs below threshold
  const smallDirs = dirItems.filter((d) => isSmallDir(d, knownTotal));
  const items = dirItems
    .filter((d) => !isSmallDir(d, knownTotal))
    .sort((a, b) => b.weight - a.weight || collator.compare(a.name, b.name));

  if (hasFilesAgg) items.push(syntheticItem(listingPath, "files", "Files", fileBytes));
  if (smallDirs.length > 0) {
    const otherBytes = smallDirs.reduce((s, d) => s + d.weight, 0);
    items.push(syntheticItem(listingPath, "other", "Other", otherBytes));
  }
  if (freeBytes !== null) items.push(syntheticItem(listingPath, "free", "Free space", freeBytes));

  return { items, bytesTotal };
}

function innerR(depth: number): number {
  const hole = OUTER_RADIUS * HOLE_RATIO;
  const band = (OUTER_RADIUS - hole) / MAX_RINGS;
  return hole + depth * band + 0.4;
}

function outerR(depth: number): number {
  const hole = OUTER_RADIUS * HOLE_RATIO;
  const band = (OUTER_RADIUS - hole) / MAX_RINGS;
  return hole + (depth + 1) * band - 0.4;
}

type LayoutContext = {
  listings: ReadonlyMap<string, Entry[]>;
  rootPath: string;
  nodes: LayoutNode[];
};

type Span = {
  path: string;
  entries: Entry[];
  startAngle: number;
  endAngle: number;
  depth: number;
  available: number | null;
  parentHue: number | null;
};

function folderHue(item: GroupItem, depth: number, parentHue: number | null): number | null {
  if (item.kind !== "folder") return null;
  return depth === 0 ? hueFromName(item.name) : parentHue;
}

function layoutNested(ctx: LayoutContext, item: GroupItem, span: Omit<Span, "path" | "entries">) {
  if (item.kind !== "folder" || item.path === null || span.depth >= MAX_RINGS) return;
  const entries = ctx.listings.get(item.path);
  if (entries) layoutChildren(ctx, { ...span, path: item.path, entries });
}

function layoutChildren(ctx: LayoutContext, span: Span): void {
  const { path, depth, available } = span;
  const includeFree =
    depth === 0 && path === ctx.rootPath && platform().isVolumeRoot(path) && available !== null;
  const { items, bytesTotal } = groupListing(span.entries, path, includeFree, available);
  if (items.length === 0) return;

  const parentSweep = span.endAngle - span.startAngle;
  const totalWeight = items.reduce((s, item) => s + item.weight, 0);
  const ir = innerR(depth);
  const or = outerR(depth);

  let angle = span.startAngle;

  for (const item of items) {
    const fraction = totalWeight > 0 ? item.weight / totalWeight : 1 / items.length;
    const end = angle + fraction * parentSweep;
    const hue = folderHue(item, depth, span.parentHue);

    ctx.nodes.push({
      id: item.id,
      name: item.name,
      path: item.path,
      depth,
      startAngle: angle,
      endAngle: end,
      innerRadius: ir,
      outerRadius: or,
      bytes: item.bytes,
      parentBytes: bytesTotal,
      kind: item.kind,
      hue,
      pending: item.pending,
      clickable: item.kind === "folder" && !item.isError,
    });

    layoutNested(ctx, item, {
      startAngle: angle,
      endAngle: end,
      depth: depth + 1,
      available: null,
      parentHue: hue,
    });

    angle = end;
  }
}

export function layoutSunburst(input: LayoutInput): LayoutNode[] {
  const { rootPath, listings, available } = input;
  const rootEntries = listings.get(rootPath);
  if (!rootEntries || rootEntries.length === 0) return [];

  const ctx: LayoutContext = { listings, rootPath, nodes: [] };
  layoutChildren(ctx, {
    path: rootPath,
    entries: rootEntries,
    startAngle: 0,
    endAngle: Math.PI * 2,
    depth: 0,
    available,
    parentHue: null,
  });
  return ctx.nodes;
}

/** Sum of all root entry bytes (no floor weight — for display). */
export function rootTotalBytes(_rootPath: string, entries: Entry[]): number {
  return sumBytes(entries);
}

export function percentOfParent(bytes: number, parentBytes: number): string {
  if (parentBytes === 0) return "0.0%";
  return `${((bytes / parentBytes) * 100).toFixed(1)}%`;
}

export function segmentAriaLabel(node: LayoutNode): string {
  if (node.pending) return `${node.name}, measuring`;
  if (node.bytes === null) return node.name;
  const pct = percentOfParent(node.bytes, node.parentBytes);
  return `${node.name}, ${formatBytes(node.bytes)}, ${pct} of parent`;
}
