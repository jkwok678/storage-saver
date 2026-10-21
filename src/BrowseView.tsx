import { useEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { FileIcon, FolderIcon } from "./icons";
import {
  breadcrumbs,
  errorText,
  formatBytes,
  formatDate,
  kindOf,
  runningInApp,
} from "./format";

type Entry = {
  name: string;
  path: string;
  isDir: boolean;
  bytes: number | null;
  modified: number | null;
  error: string | null;
};

type ScanEvent =
  | { event: "started"; data: { entries: Entry[] } }
  | { event: "sized"; data: { path: string; bytes: number; error: string | null } }
  | { event: "done" };

type SortKey = "name" | "modified" | "size" | "kind";

type Sort = {
  key: SortKey;
  descending: boolean;
};

type Props = {
  initialPath: string;
  onPathChange: (path: string) => void;
  available: number | null;
};

const COLUMNS: Array<{ key: SortKey; label: string }> = [
  { key: "name", label: "Name" },
  { key: "modified", label: "Date Modified" },
  { key: "size", label: "Size" },
  { key: "kind", label: "Kind" },
];

function BrowseView({ initialPath, onPathChange, available }: Props) {
  const [request, setRequest] = useState({ path: initialPath, refresh: false });
  const [entries, setEntries] = useState<Entry[]>([]);
  const [scanning, setScanning] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);
  const [sort, setSort] = useState<Sort>({ key: "name", descending: false });
  const [selected, setSelected] = useState<string | null>(null);
  const scanGen = useRef(0);
  const path = request.path;

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
    setSelected(null);

    if (!runningInApp()) {
      setScanError("Open Storage Saver to read this disk.");
      setScanning(false);
      return;
    }

    const channel = new Channel<ScanEvent>();
    channel.onmessage = (message) => {
      if (scanGen.current !== gen) return;
      if (message.event === "started") {
        setEntries(message.data.entries);
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
    invoke("scan_directory", { path: request.path, refresh: request.refresh, onEvent: channel })
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
  }, [request]);

  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      // Deferred so a StrictMode remount does not cancel the scan it just started.
      queueMicrotask(() => {
        if (!mounted.current && runningInApp()) invoke("stop_scan").catch(() => {});
      });
    };
  }, []);

  const open = (next: string) => {
    setRequest({ path: next, refresh: false });
    onPathChange(next);
  };

  const toggleSort = (key: SortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, descending: !current.descending }
        : { key, descending: key !== "name" },
    );
  };

  const rows = useMemo(() => {
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    const direction = sort.descending ? -1 : 1;
    return [...entries].sort((a, b) => {
      const folderOrder = Number(b.isDir) - Number(a.isDir);
      if (folderOrder !== 0) return folderOrder;

      let order = 0;
      switch (sort.key) {
        case "name":
          order = collator.compare(a.name, b.name);
          break;
        case "modified":
          order = (a.modified ?? 0) - (b.modified ?? 0);
          break;
        case "size":
          order = (a.bytes ?? 0) - (b.bytes ?? 0);
          break;
        case "kind":
          order = collator.compare(kindOf(a.name, a.isDir), kindOf(b.name, b.isDir));
          break;
      }
      return (order || collator.compare(a.name, b.name)) * direction;
    });
  }, [entries, sort]);

  const measuredCount = entries.filter((entry) => entry.bytes !== null || entry.error).length;
  const selectedEntry = entries.find((entry) => entry.path === selected) ?? null;
  const crumbs = breadcrumbs(path);

  return (
    <section className="browse">
      <div className="location">
        <nav className="crumbs" aria-label="Folder">
          {crumbs.map((crumb, index) => (
            <span key={crumb.path} className="crumb">
              {index > 0 && <span className="sep">/</span>}
              {crumb.path === path ? (
                <span className="current">{crumb.label}</span>
              ) : (
                <button type="button" onClick={() => open(crumb.path)}>
                  {crumb.label}
                </button>
              )}
            </span>
          ))}
        </nav>
        <div className="actions">
          <p className="status" aria-live="polite">
            {scanError
              ? scanError
              : scanning
                ? entries.length === 0
                  ? "Reading this folder…"
                  : `Measuring ${measuredCount} of ${entries.length}`
                : `${entries.length} ${entries.length === 1 ? "item" : "items"}`}
          </p>
          <button
            type="button"
            className="refresh"
            onClick={() => setRequest({ path, refresh: true })}
            disabled={scanning}
          >
            Refresh
          </button>
        </div>
      </div>

      <div className="finder">
        <div className="finder-scroll">
          <table className="finder-table">
            <colgroup>
              <col className="col-name" />
              <col className="col-modified" />
              <col className="col-size" />
              <col className="col-kind" />
            </colgroup>
            <thead>
              <tr>
                {COLUMNS.map((column) => (
                  <th
                    key={column.key}
                    scope="col"
                    aria-sort={
                      sort.key === column.key
                        ? sort.descending
                          ? "descending"
                          : "ascending"
                        : "none"
                    }
                  >
                    <button
                      type="button"
                      className={sort.key === column.key ? "sorted" : undefined}
                      onClick={() => toggleSort(column.key)}
                    >
                      <span>{column.label}</span>
                      {sort.key === column.key && (
                        <span className="sort-arrow" aria-hidden>
                          {sort.descending ? "▾" : "▴"}
                        </span>
                      )}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((entry) => {
                const openable = entry.isDir && !entry.error;
                return (
                  <tr
                    key={entry.path}
                    className={selected === entry.path ? "selected" : undefined}
                    tabIndex={0}
                    aria-selected={selected === entry.path}
                    title={entry.path}
                    onClick={() => setSelected(entry.path)}
                    onDoubleClick={() => openable && open(entry.path)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" && openable) {
                        event.preventDefault();
                        open(entry.path);
                      }
                    }}
                  >
                    <th scope="row" className="name-cell">
                      <span className="entry-icon">
                        {entry.isDir ? <FolderIcon /> : <FileIcon />}
                      </span>
                      <span className="name">{entry.name}</span>
                      {openable && (
                        <span className="chevron" aria-hidden>
                          ›
                        </span>
                      )}
                    </th>
                    <td className="modified-cell">{formatDate(entry.modified)}</td>
                    <td className={entry.error ? "size-cell error" : "size-cell"}>
                      {sizeText(entry)}
                    </td>
                    <td className="kind-cell">{kindOf(entry.name, entry.isDir)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {entries.length === 0 && !scanning && !scanError && (
            <p className="empty">This folder is empty.</p>
          )}
        </div>
        <footer className="finder-status">
          <span>
            {entries.length} {entries.length === 1 ? "item" : "items"}
            {selectedEntry ? `, ${selectedEntry.name} selected` : ""}
          </span>
          {available !== null && <span>{formatBytes(available)} available</span>}
        </footer>
      </div>
    </section>
  );
}

function sizeText(entry: Entry): string {
  if (entry.error) return entry.error;
  if (entry.bytes === null) return "Measuring…";
  return formatBytes(entry.bytes);
}

export default BrowseView;
