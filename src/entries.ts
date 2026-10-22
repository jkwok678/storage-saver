import { formatBytes, kindOf } from "./format";

export type Entry = {
  name: string;
  path: string;
  isDir: boolean;
  bytes: number | null;
  modified: number | null;
  error: string | null;
};

export type SortKey = "name" | "modified" | "size" | "kind";

export type Sort = {
  key: SortKey;
  descending: boolean;
};

function compareBy(key: SortKey, a: Entry, b: Entry, collator: Intl.Collator): number {
  switch (key) {
    case "name":
      return collator.compare(a.name, b.name);
    case "modified":
      return (a.modified ?? 0) - (b.modified ?? 0);
    case "size":
      return (a.bytes ?? 0) - (b.bytes ?? 0);
    case "kind":
      return collator.compare(kindOf(a.name, a.isDir), kindOf(b.name, b.isDir));
  }
}

/** Folders first, then by the sort key, falling back to name. */
export function sortEntries(entries: Entry[], sort: Sort): Entry[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  const direction = sort.descending ? -1 : 1;
  return [...entries].sort((a, b) => {
    const folderOrder = Number(b.isDir) - Number(a.isDir);
    if (folderOrder !== 0) return folderOrder;
    const order = compareBy(sort.key, a, b, collator) || collator.compare(a.name, b.name);
    return order * direction;
  });
}

/** Clicking the active column flips it; a new column starts descending, except Name. */
export function nextSort(current: Sort, key: SortKey): Sort {
  if (current.key === key) return { key, descending: !current.descending };
  return { key, descending: key !== "name" };
}

export function sizeText(entry: Entry): string {
  if (entry.error) return entry.error;
  if (entry.bytes === null) return "Measuring…";
  return formatBytes(entry.bytes);
}

export function itemCount(count: number): string {
  return `${count} ${count === 1 ? "item" : "items"}`;
}

export function scanStatus(entries: Entry[], scanning: boolean, scanError: string | null): string {
  if (scanError) return scanError;
  if (!scanning) return itemCount(entries.length);
  if (entries.length === 0) return "Reading this folder…";
  const measured = entries.filter((entry) => entry.bytes !== null || entry.error).length;
  return `Measuring ${measured} of ${entries.length}`;
}
