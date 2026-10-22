export function formatBytes(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(value)} bytes`;
  const digits = value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Could not read this folder";
}

export function runningInApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function breadcrumbs(path: string): Array<{ label: string; path: string }> {
  const crumbs = [{ label: "This Mac", path: "/" }];
  let current = "";
  for (const part of path.split("/").filter(Boolean)) {
    current += `/${part}`;
    crumbs.push({ label: part, path: current });
  }
  return crumbs;
}

const dateFormat = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

export function formatDate(seconds: number | null): string {
  if (seconds === null) return "—";
  return dateFormat.format(new Date(seconds * 1000));
}

const EXTENSION_KINDS: Record<string, string> = {
  avi: "AVI movie",
  bmp: "Bitmap image",
  csv: "CSV document",
  dmg: "Disk image",
  doc: "Word document",
  docx: "Word document",
  gif: "GIF image",
  gz: "Gzip archive",
  heic: "HEIC image",
  jpeg: "JPEG image",
  jpg: "JPEG image",
  js: "JavaScript file",
  json: "JSON document",
  m4a: "AAC audio",
  m4v: "MPEG-4 movie",
  md: "Markdown document",
  mov: "QuickTime movie",
  mp3: "MP3 audio",
  mp4: "MPEG-4 movie",
  pdf: "PDF document",
  png: "PNG image",
  ppt: "PowerPoint presentation",
  pptx: "PowerPoint presentation",
  svg: "SVG image",
  tar: "Tar archive",
  tif: "TIFF image",
  tiff: "TIFF image",
  ts: "TypeScript file",
  tsx: "TypeScript file",
  txt: "Text document",
  wav: "WAV audio",
  webp: "Web image",
  xls: "Excel spreadsheet",
  xlsx: "Excel spreadsheet",
  zip: "ZIP archive",
};

export function kindOf(name: string, isDir: boolean): string {
  if (isDir) return "Folder";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return name.startsWith(".") ? "Hidden file" : "Document";
  const extension = name.slice(dot + 1).toLowerCase();
  if (!extension) return "Document";
  return EXTENSION_KINDS[extension] ?? `${extension.toUpperCase()} file`;
}

export function parentPath(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash <= 0 ? "/" : path.slice(0, slash);
}
