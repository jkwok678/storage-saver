import { useEffect, useMemo, useRef, useState } from "react";
import {
  type Entry,
  itemCount,
  nextSort,
  type Sort,
  type SortKey,
  scanStatus,
  sizeText,
  sortEntries,
} from "./entries";
import {
  breadcrumbs,
  formatBytes,
  formatDate,
  isWithin,
  kindOf,
  parentPath,
  type TrashedItem,
} from "./format";
import { FileIcon, FolderIcon } from "./icons";
import { type ScanRequest, useDirectoryScan } from "./useDirectoryScan";

type Props = {
  initialPath: string;
  onPathChange: (path: string) => void;
  available: number | null;
  trashed?: TrashedItem | null;
};

const COLUMNS: Array<{ key: SortKey; label: string }> = [
  { key: "name", label: "Name" },
  { key: "modified", label: "Date Modified" },
  { key: "size", label: "Size" },
  { key: "kind", label: "Kind" },
];

function BrowseView({ initialPath, onPathChange, available, trashed }: Props) {
  const [request, setRequest] = useState<ScanRequest>({ path: initialPath, refresh: false });
  const [sort, setSort] = useState<Sort>({ key: "name", descending: false });
  const [selected, setSelected] = useState<string | null>(null);
  const { entries, scanning, scanError } = useDirectoryScan(request);
  const path = request.path;

  const scan = (next: ScanRequest) => {
    setSelected(null);
    setRequest(next);
  };

  const open = (next: string) => {
    scan({ path: next, refresh: false });
    onPathChange(next);
  };

  // Only items trashed while this view is open; a remount already reads current totals.
  const seenTrash = useRef(trashed?.seq);
  useEffect(() => {
    if (!trashed || trashed.seq === seenTrash.current) return;
    seenTrash.current = trashed.seq;
    if (isWithin(path, trashed.path)) {
      const parent = parentPath(trashed.path);
      setSelected(null);
      setRequest({ path: parent, refresh: false });
      onPathChange(parent);
    } else if (isWithin(trashed.path, path)) {
      // The stored totals already leave the item out, so reading them again needs no walk.
      setRequest({ path, refresh: false });
    }
  }, [trashed, path, onPathChange]);

  const rows = useMemo(() => sortEntries(entries, sort), [entries, sort]);
  const selectedEntry = entries.find((entry) => entry.path === selected) ?? null;

  return (
    <section className="browse">
      <div className="location">
        <Breadcrumbs path={path} onOpen={open} />
        <div className="actions">
          <p className="status" aria-live="polite">
            {scanStatus(entries, scanning, scanError)}
          </p>
          <button
            type="button"
            className="refresh"
            onClick={() => scan({ path, refresh: true })}
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
            <SortHeader
              sort={sort}
              onSort={(key) => setSort((current) => nextSort(current, key))}
            />
            <tbody>
              {rows.map((entry) => (
                <EntryRow
                  key={entry.path}
                  entry={entry}
                  selected={selected === entry.path}
                  onSelect={setSelected}
                  onOpen={open}
                />
              ))}
            </tbody>
          </table>
          {entries.length === 0 && !scanning && !scanError && (
            <p className="empty">This folder is empty.</p>
          )}
        </div>
        <footer className="finder-status">
          <span>
            {itemCount(entries.length)}
            {selectedEntry ? `, ${selectedEntry.name} selected` : ""}
          </span>
          {available !== null && <span>{formatBytes(available)} available</span>}
        </footer>
      </div>
    </section>
  );
}

function Breadcrumbs({ path, onOpen }: { path: string; onOpen: (path: string) => void }) {
  return (
    <nav className="crumbs" aria-label="Folder">
      {breadcrumbs(path).map((crumb, index) => (
        <span key={crumb.path} className="crumb">
          {index > 0 && <span className="sep">/</span>}
          {crumb.path === path ? (
            <span className="current">{crumb.label}</span>
          ) : (
            <button type="button" onClick={() => onOpen(crumb.path)}>
              {crumb.label}
            </button>
          )}
        </span>
      ))}
    </nav>
  );
}

function SortHeader({ sort, onSort }: { sort: Sort; onSort: (key: SortKey) => void }) {
  const direction = sort.descending ? "descending" : "ascending";
  return (
    <thead>
      <tr>
        {COLUMNS.map((column) => {
          const active = sort.key === column.key;
          return (
            <th key={column.key} scope="col" aria-sort={active ? direction : "none"}>
              <button
                type="button"
                className={active ? "sorted" : undefined}
                onClick={() => onSort(column.key)}
              >
                <span>{column.label}</span>
                {active && (
                  <span className="sort-arrow" aria-hidden>
                    {sort.descending ? "▾" : "▴"}
                  </span>
                )}
              </button>
            </th>
          );
        })}
      </tr>
    </thead>
  );
}

type EntryRowProps = {
  entry: Entry;
  selected: boolean;
  onSelect: (path: string) => void;
  onOpen: (path: string) => void;
};

function EntryRow({ entry, selected, onSelect, onOpen }: EntryRowProps) {
  const openable = entry.isDir && !entry.error;
  return (
    <tr
      className={selected ? "selected" : undefined}
      tabIndex={0}
      aria-selected={selected}
      title={entry.path}
      onClick={() => onSelect(entry.path)}
      onDoubleClick={() => openable && onOpen(entry.path)}
      onKeyDown={(event) => {
        if (event.key === "Enter" && openable) {
          event.preventDefault();
          onOpen(entry.path);
        }
      }}
    >
      <th scope="row" className="name-cell">
        <span className="entry-icon">{entry.isDir ? <FolderIcon /> : <FileIcon />}</span>
        <span className="name">{entry.name}</span>
        {openable && (
          <span className="chevron" aria-hidden>
            ›
          </span>
        )}
      </th>
      <td className="modified-cell">{formatDate(entry.modified)}</td>
      <td className={entry.error ? "size-cell error" : "size-cell"}>{sizeText(entry)}</td>
      <td className="kind-cell">{kindOf(entry.name, entry.isDir)}</td>
    </tr>
  );
}

export default BrowseView;
