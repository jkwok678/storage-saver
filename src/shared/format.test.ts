import { describe, expect, it } from "vitest";
import { breadcrumbs, errorText, formatBytes, formatDate, kindOf, parentPath } from "./format";

describe("formatBytes", () => {
  it("keeps whole bytes below 1 KB", () => {
    expect(formatBytes(0)).toBe("0 bytes");
    expect(formatBytes(1023)).toBe("1023 bytes");
  });

  it("uses two decimals below 10 and one above", () => {
    expect(formatBytes(1536)).toBe("1.50 KB");
    expect(formatBytes(15 * 1024 * 1024)).toBe("15.0 MB");
  });

  it("caps at PB", () => {
    expect(formatBytes(1024 ** 6)).toBe("1024.0 PB");
  });
});

describe("errorText", () => {
  it("passes strings through and unwraps Errors", () => {
    expect(errorText("denied")).toBe("denied");
    expect(errorText(new Error("boom"))).toBe("boom");
  });

  it("falls back for anything else", () => {
    expect(errorText({ code: 1 })).toBe("Could not read this folder");
  });
});

describe("breadcrumbs", () => {
  it("starts at This Mac and accumulates each segment", () => {
    expect(breadcrumbs("/Users/me")).toEqual([
      { label: "This Mac", path: "/" },
      { label: "Users", path: "/Users" },
      { label: "me", path: "/Users/me" },
    ]);
  });

  it("is just the root for /", () => {
    expect(breadcrumbs("/")).toEqual([{ label: "This Mac", path: "/" }]);
  });
});

describe("formatDate", () => {
  it("shows a dash for unknown dates", () => {
    expect(formatDate(null)).toBe("—");
  });

  it("formats epoch seconds", () => {
    expect(formatDate(0)).toMatch(/19(69|70)/);
  });
});

describe("kindOf", () => {
  it.each([
    ["Photos", true, "Folder"],
    [".zshrc", false, "Hidden file"],
    ["Makefile", false, "Document"],
    ["trailing.", false, "Document"],
    ["clip.MOV", false, "QuickTime movie"],
    ["data.parquet", false, "PARQUET file"],
  ])("%s → %s", (name, isDir, kind) => {
    expect(kindOf(name, isDir)).toBe(kind);
  });
});

describe("parentPath", () => {
  it("returns the containing folder", () => {
    expect(parentPath("/Users/me/file.txt")).toBe("/Users/me");
  });

  it("returns / for top-level items and bare names", () => {
    expect(parentPath("/file.txt")).toBe("/");
    expect(parentPath("file.txt")).toBe("/");
  });
});
