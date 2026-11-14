import { useEffect, useRef, useState } from "react";
import { AiAdvisorPanel } from "../ai/AiAdvisorPanel";
import { type AiProviders, useAiProviders } from "../ai/aiProviders";
import BrowseView from "../browse/BrowseView";
import DiskMapView from "../diskmap/DiskMapView";
import LargestFilesView from "../largest/LargestFilesView";
import SettingsView from "../settings/SettingsView";
import { formatBytes, type TrashedItem } from "../shared/format";
import { GearIcon, SparklesIcon } from "../shared/icons";
import { platform } from "../shared/platform";
import { type DiskSummary, useDiskSummary } from "./useDiskSummary";
import "./App.css";

type View = "browse" | "largest" | "diskmap" | "settings";

const VIEWS: Array<{ key: View; label: string }> = [
  { key: "browse", label: "Browse" },
  { key: "diskmap", label: "Disk Map" },
  { key: "largest", label: "Largest Files" },
];

function App() {
  const [browsePath, setBrowsePath] = useState(() => platform().rootPath);
  const volume = platform().volumeOf(browsePath);
  const { summary, error, refresh: refreshSummary } = useDiskSummary(volume);
  const [view, setView] = useState<View>("browse");
  const [aiOpen, setAiOpen] = useState(false);
  // Mounted on first open and kept, so closing the panel keeps the conversation.
  const [aiStarted, setAiStarted] = useState(false);
  const [trashed, setTrashed] = useState<TrashedItem | null>(null);
  const ai = useAiProviders();
  const { refresh: refreshAi, load: loadAi } = ai;

  // Accounts may have changed in the terminal since the last look.
  useEffect(() => {
    if (view === "settings") refreshAi();
  }, [view, refreshAi]);

  useEffect(() => {
    if (aiStarted) loadAi();
  }, [aiStarted, loadAi]);

  useSettingsShortcut(() => setView("settings"));

  const handleItemTrashed = (path: string) => {
    refreshSummary();
    setTrashed((prev) => ({ path, seq: (prev?.seq ?? 0) + 1 }));
  };

  const toggleAi = () => {
    setAiStarted(true);
    setAiOpen(!aiOpen);
  };

  return (
    <main className="app">
      <header className="summary">
        <div className="summary-head">
          <h1>Storage</h1>
          <HeaderControls view={view} onView={setView} aiOpen={aiOpen} onToggleAi={toggleAi} />
        </div>
        <DiskStatus summary={summary} error={error} volume={volume} />
      </header>

      <div className={`app-workspace ${aiOpen ? "ai-open" : ""}`}>
        <div className="main-content">
          <CurrentView
            view={view}
            ai={ai}
            browsePath={browsePath}
            onBrowsePath={setBrowsePath}
            available={summary?.available ?? null}
            trashed={trashed}
          />
        </div>

        {aiStarted && (
          <AiAdvisorPanel
            ai={ai}
            hidden={!aiOpen}
            currentPath={browsePath}
            onClose={() => setAiOpen(false)}
            onOpenSettings={() => setView("settings")}
            onItemTrashed={handleItemTrashed}
          />
        )}
      </div>
    </main>
  );
}

type CurrentViewProps = {
  view: View;
  ai: AiProviders;
  browsePath: string;
  onBrowsePath: (path: string) => void;
  available: number | null;
  trashed: TrashedItem | null;
};

function CurrentView({ view, ai, browsePath, onBrowsePath, available, trashed }: CurrentViewProps) {
  if (view === "settings") return <SettingsView ai={ai} />;
  if (view === "largest") return <LargestFilesView trashed={trashed} />;
  if (view === "diskmap")
    return (
      <DiskMapView
        initialPath={browsePath}
        onPathChange={onBrowsePath}
        available={available}
        trashed={trashed}
      />
    );
  return (
    <BrowseView
      initialPath={browsePath}
      onPathChange={onBrowsePath}
      available={available}
      trashed={trashed}
    />
  );
}

type DiskStatusProps = { summary: DiskSummary | null; error: string | null; volume: string };

function DiskStatus({ summary, error, volume }: DiskStatusProps) {
  if (summary) return <Capacity summary={summary} volume={volume} />;
  return <p className={error ? "error" : "status"}>{error ?? "Reading disk…"}</p>;
}

/** ⌘, (Ctrl+, on Windows) opens Settings. */
function useSettingsShortcut(openSettings: () => void) {
  const open = useRef(openSettings);
  open.current = openSettings;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (platform().isSettingsShortcut(event)) {
        event.preventDefault();
        open.current();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}

type HeaderControlsProps = {
  view: View;
  onView: (view: View) => void;
  aiOpen: boolean;
  onToggleAi: () => void;
};

function HeaderControls({ view, onView, aiOpen, onToggleAi }: HeaderControlsProps) {
  return (
    <div className="header-controls">
      <nav className="views" aria-label="Views">
        {VIEWS.map(({ key, label }) => (
          <button key={key} type="button" aria-pressed={view === key} onClick={() => onView(key)}>
            {label}
          </button>
        ))}
      </nav>
      <button
        type="button"
        className={`ai-toggle-btn ${aiOpen ? "active" : ""}`}
        aria-pressed={aiOpen}
        onClick={onToggleAi}
        title={aiOpen ? "Collapse AI Advisor" : "Open AI Advisor"}
      >
        <SparklesIcon />
        <span>AI Advisor</span>
      </button>
      <button
        type="button"
        className="settings-toggle-btn"
        aria-pressed={view === "settings"}
        aria-label="Settings"
        onClick={() => onView(view === "settings" ? "browse" : "settings")}
        title={`Settings (${platform().shortcutLabel})`}
      >
        <GearIcon />
      </button>
    </div>
  );
}

function Capacity({ summary, volume }: { summary: DiskSummary; volume: string }) {
  const usedPct = summary.total > 0 ? (summary.used / summary.total) * 100 : 0;
  return (
    <>
      <meter
        className="capacity-track"
        min={0}
        max={summary.total}
        value={summary.used}
        aria-label={platform().diskLabel(volume)}
      />
      <p className="capacity-label">
        {Math.round(usedPct)}% full · {formatBytes(summary.used)} used ·{" "}
        {formatBytes(summary.available)} free
      </p>
    </>
  );
}

export default App;
