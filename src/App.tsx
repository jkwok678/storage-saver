import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import BrowseView from "./BrowseView";
import LargestFilesView from "./LargestFilesView";
import { errorText, formatBytes, runningInApp } from "./format";
import "./App.css";

type DiskSummary = {
  total: number;
  available: number;
  used: number;
};

type View = "browse" | "largest";

function App() {
  const [summary, setSummary] = useState<DiskSummary | null>(null);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const [browsePath, setBrowsePath] = useState("/");
  const [view, setView] = useState<View>("browse");

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

  const usedPct = summary && summary.total > 0 ? (summary.used / summary.total) * 100 : 0;

  return (
    <main className="app">
      <header className="summary">
        <div className="summary-head">
          <h1>Storage</h1>
          <nav className="views" aria-label="Views">
            <button
              type="button"
              aria-pressed={view === "browse"}
              onClick={() => setView("browse")}
            >
              Browse
            </button>
            <button
              type="button"
              aria-pressed={view === "largest"}
              onClick={() => setView("largest")}
            >
              Largest Files
            </button>
          </nav>
        </div>
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

      {view === "browse" ? (
        <BrowseView
          initialPath={browsePath}
          onPathChange={setBrowsePath}
          available={summary?.available ?? null}
        />
      ) : (
        <LargestFilesView />
      )}
    </main>
  );
}

export default App;
