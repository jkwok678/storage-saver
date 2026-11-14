import { describe, expect, it } from "vitest";
import { platform } from "./platform";
import { useWindowsUserAgent } from "./testPlatform";

type Crumb = { label: string; path: string };

const crumbs = (...pairs: Array<[string, string]>): Crumb[] =>
  pairs.map(([label, path]) => ({ label, path }));

const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { key: ",", ...init });

describe("platform on macOS", () => {
  it("describes the Mac", () => {
    const p = platform();
    expect([p.os, p.rootPath, p.rootLabel, p.machine, p.fileManager, p.trashName]).toEqual([
      "macos",
      "/",
      "This Mac",
      "Mac",
      "Finder",
      "Trash",
    ]);
    expect(p.diskLabel("/")).toBe("Boot disk usage");
    expect(p.starterPrompts).toContain("Find developer caches, DerivedData, and build artifacts");
    expect(p.restoreHint(true)).toBe("Restore it with Put Back in Finder.");
    expect(p.restoreHint(false)).toMatch(/^Put Back is unavailable/);
  });

  it.each([
    ["/", [["This Mac", "/"]]],
    [
      "/Users/me",
      [
        ["This Mac", "/"],
        ["Users", "/Users"],
        ["me", "/Users/me"],
      ],
    ],
  ] as Array<[string, Array<[string, string]>]>)("breadcrumbs of %s", (path, expected) => {
    expect(platform().breadcrumbs(path)).toEqual(crumbs(...expected));
  });

  it.each([
    ["/Users/me", "/Users"],
    ["/Users", "/"],
    ["/", "/"],
  ])("parent of %s is %s", (path, parent) => {
    expect(platform().parentPath(path)).toBe(parent);
  });

  it.each([
    ["/a/b", "/a", true],
    ["/a", "/a", true],
    ["/ab", "/a", false],
    ["/a", "/", true],
    ["/A/b", "/a", false],
  ])("isWithin(%s, %s) is %s", (path, ancestor, expected) => {
    expect(platform().isWithin(path, ancestor)).toBe(expected);
  });

  it.each([
    ["/", true],
    ["/Users", false],
  ])("isVolumeRoot(%s) is %s", (path, expected) => {
    expect(platform().isVolumeRoot(path)).toBe(expected);
  });

  it("has one volume and no virtual root", () => {
    expect(platform().volumeOf("/Users/me")).toBe("/");
    expect(platform().isVirtualRoot("")).toBe(false);
  });

  it.each([
    [{ metaKey: true }, true],
    [{ ctrlKey: true }, false],
    [{ metaKey: true, key: "." }, false],
  ])("settings shortcut %j is %s", (init, expected) => {
    expect(platform().isSettingsShortcut(key(init))).toBe(expected);
    expect(platform().shortcutLabel).toBe("⌘,");
  });
});

describe("platform on Windows", () => {
  useWindowsUserAgent();

  it("describes the PC", () => {
    const p = platform();
    expect([p.os, p.rootPath, p.rootLabel, p.machine, p.fileManager, p.trashName]).toEqual([
      "windows",
      "",
      "This PC",
      "PC",
      "File Explorer",
      "Recycle Bin",
    ]);
    expect(p.diskLabel("")).toBe("This PC disk usage");
    expect(p.diskLabel("C:\\")).toBe("Disk usage of C:");
    expect(p.starterPrompts).toContain("Find developer caches, temp files, and build artifacts");
    expect(p.restoreHint(true)).toBe("Restore it from the Recycle Bin.");
    expect(p.restoreHint(false)).toMatch(/Recycle Bin/);
  });

  it.each([
    ["", [["This PC", ""]]],
    [
      "C:\\",
      [
        ["This PC", ""],
        ["C:", "C:\\"],
      ],
    ],
    [
      "C:\\Users\\me",
      [
        ["This PC", ""],
        ["C:", "C:\\"],
        ["Users", "C:\\Users"],
        ["me", "C:\\Users\\me"],
      ],
    ],
  ] as Array<[string, Array<[string, string]>]>)("breadcrumbs of %j", (path, expected) => {
    expect(platform().breadcrumbs(path)).toEqual(crumbs(...expected));
  });

  it.each([
    ["C:\\Users\\me", "C:\\Users"],
    ["C:\\Users", "C:\\"],
    ["C:\\", ""],
    ["", ""],
  ])("parent of %j is %j", (path, parent) => {
    expect(platform().parentPath(path)).toBe(parent);
  });

  it.each([
    ["C:\\a\\b", "C:\\a", true],
    ["c:\\A\\B", "C:\\a", true],
    ["C:\\a", "C:\\a", true],
    ["C:\\ab", "C:\\a", false],
    ["C:\\a", "C:\\", true],
    ["D:\\a", "C:\\", false],
    ["C:\\a", "", true],
    ["", "", true],
    ["", "C:\\", false],
  ])("isWithin(%j, %j) is %s", (path, ancestor, expected) => {
    expect(platform().isWithin(path, ancestor)).toBe(expected);
  });

  it.each([
    ["C:\\", true],
    ["d:\\", true],
    ["C:\\Users", false],
    ["", false],
  ])("isVolumeRoot(%j) is %s", (path, expected) => {
    expect(platform().isVolumeRoot(path)).toBe(expected);
  });

  it.each([
    ["C:\\Users\\me", "C:\\"],
    ["D:\\", "D:\\"],
    ["", ""],
  ])("volumeOf(%j) is %j", (path, volume) => {
    expect(platform().volumeOf(path)).toBe(volume);
  });

  it.each([
    ["", true],
    ["C:\\", false],
  ])("isVirtualRoot(%j) is %s", (path, expected) => {
    expect(platform().isVirtualRoot(path)).toBe(expected);
  });

  it.each([
    [{ ctrlKey: true }, true],
    [{ metaKey: true }, false],
    [{ ctrlKey: true, key: "." }, false],
  ])("settings shortcut %j is %s", (init, expected) => {
    expect(platform().isSettingsShortcut(key(init))).toBe(expected);
    expect(platform().shortcutLabel).toBe("Ctrl+,");
  });
});
