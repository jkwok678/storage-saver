import { useEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import "./App.css";

type DiskSummary = {
  total: number;
  available: number;
  used: number;
};

type Entry = {
  name: string;
  path: string;
  isDir: boolean;
  bytes: number | null;
  error: string | null;
};

type ScanEvent =
  | { event: "started"; data: { entries: Array<Omit<Entry, "bytes" | "error">> } }
  | { event: "sized"; data: { path: string; bytes: number; error: string | null } }
  | { event: "done" };

const MAX_ROWS = 40;

function formatBytes(bytes: number): string {
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

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Could not scan this folder";
}

function runningInApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function breadcrumbs(path: string): Array<{ label: string; path: string }> {
  const crumbs = [{ label: "This Mac", path: "/" }];
  let current = "";
  for (const part of path.split("/").filter(Boolean)) {
    current += `/${part}`;
    crumbs.push({ label: part, path: current });
  }
  return crumbs;
}

function layoutEntries(entries: Entry[]): { rows: Entry[]; smaller: Entry[] } {
  const errors: Entry[] = [];
  const pending: Entry[] = [];
  const sized: Entry[] = [];
  for (const entry of entries) {
    if (entry.error) errors.push(entry);
    else if (entry.bytes === null) pending.push(entry);
    else sized.push(entry);
  }

  const byName = (a: Entry, b: Entry) =>
    a.name.localeCompare(b.name, undefined, { numeric: true });
  errors.sort(byName);
  pending.sort((a, b) => Number(b.isDir) - Number(a.isDir) || byName(a, b));
  sized.sort((a, b) => (b.bytes ?? 0) - (a.bytes ?? 0) || byName(a, b));

  const errorSlots = Math.min(errors.length, 8);
  const sizedSlots = Math.min(sized.length, 30);
  let used = errorSlots + sizedSlots;
  const pendingSlots = Math.min(pending.length, Math.max(0, MAX_ROWS - used));
  used += pendingSlots;
  const extraSized = Math.min(sized.length - sizedSlots, Math.max(0, MAX_ROWS - used));

  const rows = [
    ...sized.slice(0, sizedSlots + extraSized),
    ...pending.slice(0, pendingSlots),
    ...errors.slice(0, errorSlots),
  ];
  const shown = new Set(rows.map((entry) => entry.path));
  return { rows, smaller: entries.filter((entry) => !shown.has(entry.path)) };
}

function App() {
  const [summary, setSummary] = useState<DiskSummary | null>(null);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const [path, setPath] = useState("/");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [scanning, setScanning] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);
  const scanGen = useRef(0);

  useEffect(() => {
    if (!runningInApp()) {
      setSummaryError("Open Storage Saver to read this disk.");
      return;
    }
    let active = true;
    invoke<DiskSummary>("disk_summary")
      .then((next) => {
        if (active) setSummary(next);
      })
      .catch((error: unknown) => {
        if (active) setSummaryError(errorText(error));
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    const gen = ++scanGen.current;
    let frame: number | null = null;
    const updates = new Map<string, { bytes: number; error: string | null }>();

    const flush = () => {
      frame = null;
      if (scanGen.current !== gen || updates.size === 0) {
        updates.clear();
        return;
      }
      const batch = new Map(updates);
      updates.clear();
      setEntries((current) =>
        current.map((entry) => {
          const update = batch.get(entry.path);
          return update ? { ...entry, bytes: update.bytes, error: update.error } : entry;
        }),
      );
    };

    setEntries([]);
    setScanning(true);
    setScanError(null);

    if (!runningInApp()) {
      setScanError("Open Storage Saver to read this disk.");
      setScanning(false);
      return;
    }

    const channel = new Channel<ScanEvent>();
    channel.onmessage = (message) => {
      if (scanGen.current !== gen) return;
      if (message.event === "started") {
        setEntries(
          message.data.entries.map((entry) => ({
            ...entry,
            bytes: null,
            error: null,
          })),
        );
        return;
      }
      if (message.event === "sized") {
        updates.set(message.data.path, {
          bytes: message.data.bytes,
          error: message.data.error,
        });
        if (frame === null) frame = requestAnimationFrame(flush);
        return;
      }
      if (frame !== null) cancelAnimationFrame(frame);
      flush();
    };

    let active = true;
    invoke("scan_directory", { path, onEvent: channel })
      .then(() => {
        if (active && scanGen.current === gen) setScanning(false);
      })
      .catch((error: unknown) => {
        if (active && scanGen.current === gen) {
          setScanError(errorText(error));
          setScanning(false);
        }
      });

    return () => {
      active = false;
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [path]);

  const crumbs = breadcrumbs(path);
  const { rows, smaller } = useMemo(() => layoutEntries(entries), [entries]);
  const smallerPending = smaller.some((entry) => entry.bytes === null && !entry.error);
  const smallerBytes = smaller.reduce((sum, entry) => sum + (entry.bytes ?? 0), 0);
  const measuredBytes = entries.reduce((sum, entry) => sum + (entry.bytes ?? 0), 0);
  const otherBytes =
    path === "/" && summary && !scanning && !scanError
      ? Math.max(0, summary.used - measuredBytes)
      : 0;
  const maxBytes = Math.max(
    1,
    ...rows.map((entry) => entry.bytes ?? 0),
    smallerPending ? 0 : smallerBytes,
    otherBytes,
  );
  const measuredCount = entries.filter((entry) => entry.bytes !== null || entry.error).length;
  const usedPct = summary && summary.total > 0 ? (summary.used / summary.total) * 100 : 0;

  return (
    <main className="app">
      <header className="summary">
        <h1>Storage</h1>
        {summary ? (
          <>
            <div
              className="capacity-track"
              role="meter"
              aria-valuemin={0}
              aria-valuemax={summary.total}
              aria-valuenow={summary.used}
              aria-label="Boot disk usage"
            >
              <div className="capacity-used" style={{ width: `${Math.min(usedPct, 100)}%` }} />
            </div>
            <p className="capacity-label">
              {Math.round(usedPct)}% full · {formatBytes(summary.used)} used ·{" "}
              {formatBytes(summary.available)} free
            </p>
          </>
        ) : (
          <p className={summaryError ? "error" : "status"}>
            {summaryError ?? "Reading disk…"}
          </p>
        )}
      </header>

      <div className="location">
        <nav className="crumbs" aria-label="Folder">
          {crumbs.map((crumb, index) => (
            <span key={crumb.path} className="crumb">
              {index > 0 && <span className="sep">/</span>}
              {crumb.path === path ? (
                <span className="current">{crumb.label}</span>
              ) : (
                <button type="button" onClick={() => setPath(crumb.path)}>
                  {crumb.label}
                </button>
              )}
            </span>
          ))}
        </nav>
        <p className="status">
          {scanError
            ? scanError
            : scanning
              ? entries.length === 0
                ? "Reading this folder…"
                : `Measuring ${measuredCount} of ${entries.length}`
              : `${entries.length} ${entries.length === 1 ? "item" : "items"}`}
        </p>
      </div>

      <ul className="list">
        {rows.map((entry) => {
          const openable = entry.isDir && !entry.error;
          const width =
            entry.bytes === null || entry.error ? 0 : (entry.bytes / maxBytes) * 100;
          const body = (
            <>
              <span className="name" title={entry.path}>
                {entry.name}
                {entry.isDir && !entry.error && <span className="chevron">›</span>}
              </span>
              <span className="bar-track" aria-hidden>
                {entry.bytes === null && !entry.error ? (
                  <span className="bar-fill pending" />
                ) : (
                  <span className="bar-fill" style={{ width: `${width}%` }} />
                )}
              </span>
              <span className={entry.error ? "size error" : "size"}>{sizeText(entry)}</span>
            </>
          );
          return (
            <li key={entry.path}>
              {openable ? (
                <button type="button" className="row" onClick={() => setPath(entry.path)}>
                  {body}
                </button>
              ) : (
                <div className="row">{body}</div>
              )}
            </li>
          );
        })}
        {smaller.length > 0 && (
          <li>
            <div className="row smaller">
              <span className="name">Smaller items</span>
              <span className="bar-track" aria-hidden>
                {!smallerPending && (
                  <span
                    className="bar-fill"
                    style={{ width: `${(smallerBytes / maxBytes) * 100}%` }}
                  />
                )}
              </span>
              <span className="size">
                {smallerPending
                  ? "Measuring…"
                  : `${smaller.length} · ${formatBytes(smallerBytes)}`}
              </span>
            </div>
          </li>
        )}
        {otherBytes > 0 && (
          <li>
            <div className="row other">
              <span className="name">Other system space</span>
              <span className="bar-track" aria-hidden>
                <span
                  className="bar-fill muted"
                  style={{ width: `${(otherBytes / maxBytes) * 100}%` }}
                />
              </span>
              <span className="size">{formatBytes(otherBytes)}</span>
            </div>
          </li>
        )}
      </ul>
      {entries.length === 0 && !scanning && !scanError && (
        <p className="empty">This folder is empty.</p>
      )}
    </main>
  );
}

function sizeText(entry: Entry): string {
  if (entry.error) return entry.error;
  if (entry.bytes === null) return "Measuring…";
  return formatBytes(entry.bytes);
}

export default App;
