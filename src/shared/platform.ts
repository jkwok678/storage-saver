export type Os = "macos" | "windows";

type Crumb = { label: string; path: string };

export type Platform = {
  os: Os;
  rootPath: string;
  rootLabel: string;
  machine: string;
  fileManager: string;
  trashName: string;
  /** Where "Move to" sends an item, phrased for a tooltip. */
  trashTarget: string;
  shortcutLabel: string;
  starterPrompts: string[];
  isSettingsShortcut: (event: KeyboardEvent) => boolean;
  breadcrumbs: (path: string) => Crumb[];
  parentPath: (path: string) => string;
  /** Whether `path` is `ancestor` or sits somewhere inside it. */
  isWithin: (path: string, ancestor: string) => boolean;
  isVolumeRoot: (path: string) => boolean;
  /** The virtual top level that lists volumes instead of folders. */
  isVirtualRoot: (path: string) => boolean;
  /** The volume root that holds `path`. */
  volumeOf: (path: string) => string;
  /** Accessible name of the capacity meter for the volume at `volume`. */
  diskLabel: (volume: string) => string;
  restoreHint: (putBack: boolean) => string;
};

const COMMON_PROMPTS = [
  "What can I safely delete right now to free up space?",
  "Analyze the current folder and tell me what is taking space",
  "Explain my largest files and what can be cleaned up",
];

const MAC: Platform = {
  os: "macos",
  rootPath: "/",
  rootLabel: "This Mac",
  machine: "Mac",
  fileManager: "Finder",
  trashName: "Trash",
  trashTarget: "macOS Trash",
  shortcutLabel: "⌘,",
  starterPrompts: [...COMMON_PROMPTS, "Find developer caches, DerivedData, and build artifacts"],
  isSettingsShortcut: (event) => event.metaKey && event.key === ",",
  breadcrumbs(path) {
    const crumbs = [{ label: MAC.rootLabel, path: "/" }];
    let current = "";
    for (const part of path.split("/").filter(Boolean)) {
      current += `/${part}`;
      crumbs.push({ label: part, path: current });
    }
    return crumbs;
  },
  parentPath(path) {
    const slash = path.lastIndexOf("/");
    return slash <= 0 ? "/" : path.slice(0, slash);
  },
  isWithin(path, ancestor) {
    if (ancestor === "/") return path.startsWith("/");
    return path === ancestor || path.startsWith(`${ancestor}/`);
  },
  isVolumeRoot: (path) => path === "/",
  isVirtualRoot: () => false,
  volumeOf: () => "/",
  diskLabel: () => "Boot disk usage",
  restoreHint: (putBack) =>
    putBack
      ? "Restore it with Put Back in Finder."
      : "Put Back is unavailable: allow Storage Saver to control Finder in System Settings › Privacy & Security › Automation.",
};

const DRIVE_ROOT = /^[A-Za-z]:\\$/;

const WINDOWS: Platform = {
  os: "windows",
  rootPath: "",
  rootLabel: "This PC",
  machine: "PC",
  fileManager: "File Explorer",
  trashName: "Recycle Bin",
  trashTarget: "the Recycle Bin",
  shortcutLabel: "Ctrl+,",
  starterPrompts: [...COMMON_PROMPTS, "Find developer caches, temp files, and build artifacts"],
  isSettingsShortcut: (event) => event.ctrlKey && event.key === ",",
  breadcrumbs(path) {
    const crumbs = [{ label: WINDOWS.rootLabel, path: "" }];
    let current = "";
    for (const part of path.split("\\").filter(Boolean)) {
      current = current === "" ? `${part}\\` : `${current.replace(/\\$/, "")}\\${part}`;
      crumbs.push({ label: part, path: current });
    }
    return crumbs;
  },
  parentPath(path) {
    if (path === "" || DRIVE_ROOT.test(path)) return "";
    const parent = path.slice(0, path.lastIndexOf("\\"));
    return parent.endsWith(":") ? `${parent}\\` : parent;
  },
  isWithin(path, ancestor) {
    const lower = path.toLowerCase();
    const base = ancestor.toLowerCase();
    if (base === "" || base.endsWith("\\")) return lower.startsWith(base);
    return lower === base || lower.startsWith(`${base}\\`);
  },
  isVolumeRoot: (path) => DRIVE_ROOT.test(path),
  isVirtualRoot: (path) => path === "",
  volumeOf: (path) => (/^[A-Za-z]:/.test(path) ? `${path.slice(0, 2)}\\` : ""),
  diskLabel: (volume) =>
    volume === "" ? "This PC disk usage" : `Disk usage of ${volume.replace(/\\$/, "")}`,
  restoreHint: (putBack) =>
    putBack
      ? "Restore it from the Recycle Bin."
      : "Restoring is unavailable: this item could not be placed in the Recycle Bin with its restore information.",
};

/** The platform the app runs on, read from the user agent on each call. */
export function platform(): Platform {
  return /Windows/.test(navigator.userAgent) ? WINDOWS : MAC;
}
