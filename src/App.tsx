import { useState } from "react";
import BrowseView from "./BrowseView";
import { formatBytes } from "./format";
import LargestFilesView from "./LargestFilesView";
import { type DiskSummary, useDiskSummary } from "./useDiskSummary";
import "./App.css";

type View = "browse" | "largest";

const VIEWS: Array<{ key: View; label: string }> = [
  { key: "browse", label: "Browse" },
  { key: "largest", label: "Largest Files" },
];

function App() {
  const { summary, error } = useDiskSummary();
  const [browsePath, setBrowsePath] = useState("/");
  const [view, setView] = useState<View>("browse");

  return (
    <main className="app">
      <header className="summary">
        <div className="summary-head">
          <h1>Storage</h1>
          <nav className="views" aria-label="Views">
            {VIEWS.map(({ key, label }) => (
              <button
                key={key}
                type="button"
                aria-pressed={view === key}
                onClick={() => setView(key)}
              >
                {label}
              </button>
            ))}
          </nav>
        </div>
        {summary ? (
          <Capacity summary={summary} />
        ) : (
          <p className={error ? "error" : "status"}>{error ?? "Reading disk…"}</p>
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

function Capacity({ summary }: { summary: DiskSummary }) {
  const usedPct = summary.total > 0 ? (summary.used / summary.total) * 100 : 0;
  return (
    <>
      <meter
        className="capacity-track"
        min={0}
        max={summary.total}
        value={summary.used}
        aria-label="Boot disk usage"
      />
      <p className="capacity-label">
        {Math.round(usedPct)}% full · {formatBytes(summary.used)} used ·{" "}
        {formatBytes(summary.available)} free
      </p>
    </>
  );
}

export default App;
