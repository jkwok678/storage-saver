import { Channel, invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";
import { errorText, formatBytes, runningInApp } from "./format";
import { FileIcon } from "./icons";

type LargestFile = {
  name: string;
  path: string;
  bytes: number;
};

type LargestEvent =
  | { event: "batch"; data: { files: LargestFile[]; scanned: number } }
  | { event: "done"; data: { scanned: number } };

const LIMIT = 100;

function parentPath(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash <= 0 ? "/" : path.slice(0, slash);
}

function LargestFilesView() {
  const [files, setFiles] = useState<LargestFile[]>([]);
  const [scanned, setScanned] = useState(0);
  const [scanning, setScanning] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [run, setRun] = useState(0);
  const scanGen = useRef(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `run` is a counter that re-triggers the scan
  useEffect(() => {
    const gen = ++scanGen.current;
    setFiles([]);
    setScanned(0);
    setScanning(true);
    setError(null);
    setActionError(null);

    if (!runningInApp()) {
      setError("Open Storage Saver to read this disk.");
      setScanning(false);
      return;
    }

    const channel = new Channel<LargestEvent>();
    channel.onmessage = (message) => {
      if (scanGen.current !== gen) return;
      if (message.event === "batch") {
        setFiles(message.data.files);
        setScanned(message.data.scanned);
        return;
      }
      setScanned(message.data.scanned);
      setScanning(false);
    };

    invoke("scan_largest_files", { path: "/", limit: LIMIT, onEvent: channel })
      .then(() => {
        if (scanGen.current === gen) setScanning(false);
      })
      .catch((reason: unknown) => {
        if (scanGen.current === gen) {
          setError(errorText(reason));
          setScanning(false);
        }
      });

    return () => {
      scanGen.current += 1;
    };
  }, [run]);

  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      // Deferred so a StrictMode remount does not cancel the scan it just started.
      queueMicrotask(() => {
        if (!mounted.current && runningInApp()) {
          invoke("stop_largest_scan").catch(() => {});
        }
      });
    };
  }, []);

  const stop = () => {
    scanGen.current += 1;
    setScanning(false);
    if (runningInApp()) invoke("stop_largest_scan").catch(() => {});
  };

  const reveal = (path: string) => {
    if (!runningInApp()) return;
    revealItemInDir(path).catch((reason: unknown) => setActionError(errorText(reason)));
  };

  const maxBytes = files[0]?.bytes ?? 1;
  const status = error
    ? error
    : scanning
      ? `Scanning this Mac · ${scanned.toLocaleString()} files read`
      : `Top ${files.length} of ${scanned.toLocaleString()} files`;

  return (
    <section className="largest">
      <div className="location">
        <div className="largest-heading">
          <h2>Largest files</h2>
          <p className={error || actionError ? "status error" : "status"} aria-live="polite">
            {actionError ?? status}
          </p>
        </div>
        <div className="actions">
          {scanning ? (
            <button type="button" className="refresh" onClick={stop}>
              Stop
            </button>
          ) : (
            <button type="button" className="refresh" onClick={() => setRun((value) => value + 1)}>
              Scan again
            </button>
          )}
        </div>
      </div>

      <div className="largest-panel">
        <div className="largest-scroll">
          {files.length > 0 ? (
            <ol className="largest-list">
              {files.map((file, index) => (
                <li key={file.path}>
                  <button
                    type="button"
                    className="largest-row"
                    title={`${file.path}\nDouble-click or press Return to show in Finder`}
                    onDoubleClick={() => reveal(file.path)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        reveal(file.path);
                      }
                    }}
                  >
                    <span className="rank">{index + 1}</span>
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
              ))}
            </ol>
          ) : (
            <div className="largest-empty">
              <p className="status">
                {scanning ? "Looking for the biggest files…" : error ? error : "No files found."}
              </p>
            </div>
          )}
        </div>
        <footer className="finder-status">
          <span>{scanning ? "Largest files found so far" : `Largest ${files.length} files`}</span>
          <span>Double-click a file to show it in Finder</span>
        </footer>
      </div>
    </section>
  );
}

export default LargestFilesView;
