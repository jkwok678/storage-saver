import { useState } from "react";
import { AiAdvisorPanel } from "./AiAdvisorPanel";
import BrowseView from "./BrowseView";
import { formatBytes, type TrashedItem } from "./format";
import { SparklesIcon } from "./icons";
import LargestFilesView from "./LargestFilesView";
import { type DiskSummary, useDiskSummary } from "./useDiskSummary";
import "./App.css";

type View = "browse" | "largest";

const VIEWS: Array<{ key: View; label: string }> = [
  { key: "browse", label: "Browse" },
  { key: "largest", label: "Largest Files" },
];

function App() {
  const { summary, error, refresh: refreshSummary } = useDiskSummary();
  const [browsePath, setBrowsePath] = useState("/");
  const [view, setView] = useState<View>("browse");
  const [aiOpen, setAiOpen] = useState(false);
  // Mounted on first open and kept, so closing the panel keeps the conversation.
  const [aiStarted, setAiStarted] = useState(false);
  const [trashed, setTrashed] = useState<TrashedItem | null>(null);

  const handleItemTrashed = (path: string) => {
    refreshSummary();
    setTrashed((prev) => ({ path, seq: (prev?.seq ?? 0) + 1 }));
  };

  return (
    <main className="app">
      <header className="summary">
        <div className="summary-head">
          <h1>Storage</h1>
          <div className="header-controls">
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
            <button
              type="button"
              className={`ai-toggle-btn ${aiOpen ? "active" : ""}`}
              aria-pressed={aiOpen}
              onClick={() => {
                setAiStarted(true);
                setAiOpen(!aiOpen);
              }}
              title={aiOpen ? "Collapse AI Advisor" : "Open AI Advisor"}
            >
              <SparklesIcon />
              <span>AI Advisor</span>
            </button>
          </div>
        </div>
        {summary ? (
          <Capacity summary={summary} />
        ) : (
          <p className={error ? "error" : "status"}>{error ?? "Reading disk…"}</p>
        )}
      </header>

      <div className={`app-workspace ${aiOpen ? "ai-open" : ""}`}>
        <div className="main-content">
          {view === "browse" ? (
            <BrowseView
              initialPath={browsePath}
              onPathChange={setBrowsePath}
              available={summary?.available ?? null}
              trashed={trashed}
            />
          ) : (
            <LargestFilesView trashed={trashed} />
          )}
        </div>

        {aiStarted && (
          <AiAdvisorPanel
            hidden={!aiOpen}
            currentPath={browsePath}
            onClose={() => setAiOpen(false)}
            onItemTrashed={handleItemTrashed}
          />
        )}
      </div>
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
