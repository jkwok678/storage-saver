import { describe, expect, it } from "vitest";
import { type Entry, itemCount, nextSort, scanStatus, sizeText, sortEntries } from "./entries";

function entry(name: string, overrides: Partial<Entry> = {}): Entry {
  return {
    name,
    path: `/${name}`,
    isDir: false,
    bytes: null,
    modified: null,
    error: null,
    ...overrides,
  };
}

const names = (entries: Entry[]) => entries.map((item) => item.name);

describe("sortEntries", () => {
  const items = [
    entry("b.txt", { bytes: 10, modified: 3 }),
    entry("Docs", { isDir: true, bytes: 5, modified: 1 }),
    entry("a10.png", { bytes: 30, modified: 2 }),
    entry("a2.png", { bytes: 30, modified: 2 }),
  ];

  it("puts folders first and sorts names naturally", () => {
    expect(names(sortEntries(items, { key: "name", descending: false }))).toEqual([
      "Docs",
      "a2.png",
      "a10.png",
      "b.txt",
    ]);
  });

  it("keeps folders first when descending", () => {
    expect(names(sortEntries(items, { key: "size", descending: true }))).toEqual([
      "Docs",
      "a10.png",
      "a2.png",
      "b.txt",
    ]);
  });

  it("sorts by modified and kind, breaking ties by name", () => {
    expect(names(sortEntries(items, { key: "modified", descending: false }))).toEqual([
      "Docs",
      "a2.png",
      "a10.png",
      "b.txt",
    ]);
    expect(names(sortEntries(items, { key: "kind", descending: false }))).toEqual([
      "Docs",
      "a2.png",
      "a10.png",
      "b.txt",
    ]);
  });

  it("treats unmeasured sizes as zero and does not mutate the input", () => {
    const input = [entry("big", { bytes: 9 }), entry("pending")];
    expect(names(sortEntries(input, { key: "size", descending: false }))).toEqual([
      "pending",
      "big",
    ]);
    expect(names(input)).toEqual(["big", "pending"]);
  });
});

describe("nextSort", () => {
  it("flips direction on the active column", () => {
    expect(nextSort({ key: "size", descending: true }, "size")).toEqual({
      key: "size",
      descending: false,
    });
  });

  it("starts new columns descending, except name", () => {
    expect(nextSort({ key: "name", descending: false }, "size").descending).toBe(true);
    expect(nextSort({ key: "size", descending: true }, "name").descending).toBe(false);
  });
});

describe("sizeText", () => {
  it("prefers the error, then a placeholder, then the size", () => {
    expect(sizeText(entry("x", { error: "Permission denied" }))).toBe("Permission denied");
    expect(sizeText(entry("x"))).toBe("Measuring…");
    expect(sizeText(entry("x", { bytes: 2048 }))).toBe("2.00 KB");
  });
});

describe("itemCount", () => {
  it("pluralises", () => {
    expect(itemCount(1)).toBe("1 item");
    expect(itemCount(0)).toBe("0 items");
  });
});

describe("scanStatus", () => {
  const measured = [entry("a", { bytes: 1 }), entry("b", { error: "nope" }), entry("c")];

  it("shows the error first", () => {
    expect(scanStatus(measured, true, "Failed")).toBe("Failed");
  });

  it("counts items once the scan finishes", () => {
    expect(scanStatus(measured, false, null)).toBe("3 items");
  });

  it("reports progress while scanning", () => {
    expect(scanStatus([], true, null)).toBe("Reading this folder…");
    expect(scanStatus(measured, true, null)).toBe("Measuring 2 of 3");
  });
});
