import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import {
  errorText,
  formatBytes,
  parentPath,
  runningInApp,
  type TrashedItem,
} from "../shared/format";
import { FileIcon } from "../shared/icons";
import { platform } from "../shared/platform";
import { type LargestFile, useLargestScan } from "./useLargestScan";
import "./largest.css";

function scanStatus(error: string | null, scanning: boolean, scanned: number, found: number) {
  if (error) return error;
  if (scanning)
    return `Scanning this ${platform().machine} · ${scanned.toLocaleString()} files read`;
  return `Top ${found} of ${scanned.toLocaleString()} files`;
}

function emptyText(error: string | null, scanning: boolean) {
  if (scanning) return "Looking for the biggest files…";
  return error ?? "No files found.";
}

type Props = {
  trashed?: TrashedItem | null;
};

function LargestFilesView({ trashed }: Props = {}) {
  const { files, scanned, scanning, error, stop, rescan, forget } = useLargestScan();
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    if (trashed) forget(trashed.path);
  }, [trashed, forget]);

  const scanAgain = () => {
    setActionError(null);
    rescan();
  };

  const reveal = (path: string) => {
    if (!runningInApp()) return;
    revealItemInDir(path).catch((reason: unknown) => setActionError(errorText(reason)));
  };

  const maxBytes = files[0]?.bytes ?? 1;

  return (
    <section className="largest">
      <div className="location">
        <div className="largest-heading">
          <h2>Largest files</h2>
          <p className={error || actionError ? "status error" : "status"} aria-live="polite">
            {actionError ?? scanStatus(error, scanning, scanned, files.length)}
          </p>
        </div>
        <div className="actions">
          <button type="button" className="refresh" onClick={scanning ? stop : scanAgain}>
            {scanning ? "Stop" : "Scan again"}
          </button>
        </div>
      </div>

      <div className="largest-panel">
        <div className="largest-scroll">
          {files.length > 0 ? (
            <ol className="largest-list">
              {files.map((file, index) => (
                <LargestRow
                  key={file.path}
                  file={file}
                  rank={index + 1}
                  maxBytes={maxBytes}
                  onReveal={reveal}
                />
              ))}
            </ol>
          ) : (
            <div className="largest-empty">
              <p className="status">{emptyText(error, scanning)}</p>
            </div>
          )}
        </div>
        <footer className="finder-status">
          <span>{scanning ? "Largest files found so far" : `Largest ${files.length} files`}</span>
          <span>Double-click a file to show it in {platform().fileManager}</span>
        </footer>
      </div>
    </section>
  );
}

type LargestRowProps = {
  file: LargestFile;
  rank: number;
  maxBytes: number;
  onReveal: (path: string) => void;
};

function LargestRow({ file, rank, maxBytes, onReveal }: LargestRowProps) {
  return (
    <li>
      <button
        type="button"
        className="largest-row"
        title={`${file.path}\nDouble-click or press Return to show in ${platform().fileManager}`}
        onDoubleClick={() => onReveal(file.path)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onReveal(file.path);
          }
        }}
      >
        <span className="rank">{rank}</span>
        <span className="entry-icon">
          <FileIcon />
        </span>
        <span className="largest-name">
          <span className="name">{file.name}</span>
          <span className="parent">{parentPath(file.path)}</span>
        </span>
        <span className="largest-bar" aria-hidden>
          <span
            className="largest-bar-fill"
            style={{ width: `${Math.max(2, (file.bytes / maxBytes) * 100)}%` }}
          />
        </span>
        <span className="largest-size">{formatBytes(file.bytes)}</span>
      </button>
    </li>
  );
}

export default LargestFilesView;
