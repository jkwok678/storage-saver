import type { Entry } from "../browse/entries";
import { formatBytes } from "../shared/format";
import { hueFromName } from "./color";

export const MAX_RINGS = 4;
export const OTHER_THRESHOLD = 0.005;
export const MIN_EXPAND_TURN = 2 / 360;
export const VIEWBOX = 100;
export const CENTER = 50;
export const OUTER_RADIUS = 48;
export const HOLE_RATIO = 0.28;

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

function groupListing(
  entries: Entry[],
  listingPath: string,
  includeFree: boolean,
  available: number | null,
): { items: GroupItem[]; bytesTotal: number } {
  const dirs = entries.filter((e) => e.isDir);
  const files = entries.filter((e) => !e.isDir);

  const fileBytes = files.reduce((s, f) => s + (f.bytes ?? 0), 0);
  const hasFilesAgg = fileBytes > 0 || (files.length > 0 && dirs.length === 0);

  const dirItems: GroupItem[] = dirs.map((d) => {
    const isPending = d.bytes === null && d.error === null;
    const weight = isPending ? 1 : (d.bytes ?? 0);
    return {
      id: d.path,
      name: d.name,
      path: d.path,
      kind: "folder" as ArcKind,
      bytes: d.bytes,
      weight,
      pending: isPending,
      isError: d.error !== null,
    };
  });

  const dirWeight = dirItems.reduce((s, d) => s + d.weight, 0);
  const freeWeight = includeFree && available !== null ? available : 0;
  const knownTotal = dirWeight + fileBytes + freeWeight;

  // All items with actual bytes (no floor), for parentBytes display
  const bytesTotal = dirs.reduce((s, d) => s + (d.bytes ?? 0), 0) + fileBytes + freeWeight;

  // Handle all-zero: equal sweeps, pending style
  if (knownTotal === 0) {
    const allItems: GroupItem[] = [...dirItems];
    if (hasFilesAgg) {
      allItems.push({
        id: `${listingPath}::__files__`,
        name: "Files",
        path: null,
        kind: "files",
        bytes: 0,
        weight: 1,
        pending: true,
        isError: false,
      });
    }
    if (allItems.length === 0) return { items: [], bytesTotal: 0 };
    for (const item of allItems) {
      item.weight = 1;
      item.pending = true;
    }
    return { items: allItems, bytesTotal: 0 };
  }

  // Other grouping: non-pending, non-error dirs below threshold
  let otherBytes = 0;
  let otherCount = 0;
  const remainingDirs: GroupItem[] = [];

  for (const d of dirItems) {
    if (!d.pending && !d.isError && d.weight / knownTotal < OTHER_THRESHOLD) {
      otherBytes += d.weight;
      otherCount++;
    } else {
      remainingDirs.push(d);
    }
  }

  // Sort remaining dirs by weight desc, then name
  remainingDirs.sort((a, b) => b.weight - a.weight || collator.compare(a.name, b.name));

  const result: GroupItem[] = [...remainingDirs];

  if (hasFilesAgg) {
    result.push({
      id: `${listingPath}::__files__`,
      name: "Files",
      path: null,
      kind: "files",
      bytes: fileBytes,
      weight: fileBytes,
      pending: false,
      isError: false,
    });
  }

  if (otherCount > 0) {
    result.push({
      id: `${listingPath}::__other__`,
      name: "Other",
      path: null,
      kind: "other",
      bytes: otherBytes,
      weight: otherBytes,
      pending: false,
      isError: false,
    });
  }

  if (includeFree && available !== null) {
    result.push({
      id: `${listingPath}::__free__`,
      name: "Free space",
      path: null,
      kind: "free",
      bytes: available,
      weight: available,
      pending: false,
      isError: false,
    });
  }

  return { items: result, bytesTotal };
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

function layoutChildren(
  listingPath: string,
  entries: Entry[],
  startAngle: number,
  endAngle: number,
  depth: number,
  listings: ReadonlyMap<string, Entry[]>,
  available: number | null,
  rootPath: string,
  parentHue: number | null,
  nodes: LayoutNode[],
): void {
  const includeFree =
    depth === 0 && listingPath === rootPath && listingPath === "/" && available !== null;
  const { items, bytesTotal } = groupListing(entries, listingPath, includeFree, available);
  if (items.length === 0) return;

  const parentSweep = endAngle - startAngle;
  const totalWeight = items.reduce((s, item) => s + item.weight, 0);
  const ir = innerR(depth);
  const or = outerR(depth);

  let angle = startAngle;

  for (const item of items) {
    const fraction = totalWeight > 0 ? item.weight / totalWeight : 1 / items.length;
    const sweep = fraction * parentSweep;
    const end = angle + sweep;

    const hue: number | null =
      item.kind === "folder" ? (depth === 0 ? hueFromName(item.name) : parentHue) : null;

    nodes.push({
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

    if (item.kind === "folder" && item.path !== null && depth + 1 < MAX_RINGS) {
      const childListing = listings.get(item.path);
      if (childListing) {
        layoutChildren(
          item.path,
          childListing,
          angle,
          end,
          depth + 1,
          listings,
          null,
          rootPath,
          hue,
          nodes,
        );
      }
    }

    angle = end;
  }
}

export function layoutSunburst(input: LayoutInput): LayoutNode[] {
  const { rootPath, listings, available } = input;
  const rootEntries = listings.get(rootPath);
  if (!rootEntries || rootEntries.length === 0) return [];

  const nodes: LayoutNode[] = [];
  layoutChildren(
    rootPath,
    rootEntries,
    0,
    Math.PI * 2,
    0,
    listings,
    available,
    rootPath,
    null,
    nodes,
  );
  return nodes;
}

/** Sum of all root entry bytes (no floor weight — for display). */
export function rootTotalBytes(_rootPath: string, entries: Entry[]): number {
  return entries.reduce((s, e) => s + (e.bytes ?? 0), 0);
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
