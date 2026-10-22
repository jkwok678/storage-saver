import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";
import { scanStatus } from "../browse/entries";
import {
  breadcrumbs,
  errorText,
  isWithin,
  parentPath,
  runningInApp,
  type TrashedItem,
} from "../shared/format";
import "./diskmap.css";
import { layoutSunburst, rootTotalBytes } from "./layout";
import { SunburstChart } from "./SunburstChart";
import type { ScanRequest } from "./scanDirectory";
import { useSunburstScan } from "./useSunburstScan";

type Props = {
  initialPath: string;
  onPathChange: (path: string) => void;
  available: number | null;
  trashed?: TrashedItem | null;
};

function DiskMapView({ initialPath, onPathChange, available, trashed }: Props) {
  const [request, setRequest] = useState<ScanRequest>({ path: initialPath, refresh: false });
  const { listings, rootEntries, scanning, rootScanning, scanError, refresh } =
    useSunburstScan(request);
  const [actionError, setActionError] = useState<string | null>(null);
  const path = request.path;
  const offline = !runningInApp();

  const navigate = (next: string) => {
    setRequest({ path: next, refresh: false });
    onPathChange(next);
  };

  const handleRefresh = () => {
    setActionError(null);
    refresh();
  };

  const seenTrash = useRef(trashed?.seq);
  useEffect(() => {
    if (!trashed || trashed.seq === seenTrash.current) return;
    seenTrash.current = trashed.seq;
    if (isWithin(path, trashed.path)) {
      const parent = parentPath(trashed.path);
      setRequest({ path: parent, refresh: false });
      onPathChange(parent);
    } else if (isWithin(trashed.path, path)) {
      setRequest({ path, refresh: false });
    }
  }, [trashed, path, onPathChange]);

  const nodes = layoutSunburst({ rootPath: path, listings, available });
  const totalBytes = rootTotalBytes(path, rootEntries);
  const isEmpty = !offline && rootEntries.length === 0 && !scanning && !scanError;

  let statusText: string;
  if (offline) statusText = "Open Storage Saver to read this disk.";
  else if (actionError) statusText = actionError;
  else if (scanning && !rootScanning) statusText = "Reading nested folders…";
  else statusText = scanStatus(rootEntries, rootScanning, scanError);

  const reveal = (revealPath: string) => {
    revealItemInDir(revealPath).catch((reason: unknown) => setActionError(errorText(reason)));
  };

  return (
    <section className="diskmap">
      <div className="location">
        <nav className="crumbs" aria-label="Folder">
          {breadcrumbs(path).map((crumb, index) => (
            <span key={crumb.path} className="crumb">
              {index > 0 && <span className="sep">/</span>}
              {crumb.path === path ? (
                <span className="current">{crumb.label}</span>
              ) : (
                <button type="button" onClick={() => navigate(crumb.path)}>
                  {crumb.label}
                </button>
              )}
            </span>
          ))}
        </nav>
        <div className="actions">
          <p className={`status${actionError ? " error" : ""}`} aria-live="polite">
            {statusText}
          </p>
          <button
            type="button"
            className="refresh"
            onClick={handleRefresh}
            disabled={scanning || offline}
          >
            Refresh
          </button>
        </div>
      </div>

      <div className="diskmap-panel">
        {offline ? (
          <p className="status">Open Storage Saver to read this disk.</p>
        ) : scanError ? (
          <p className="status error">{scanError}</p>
        ) : isEmpty ? (
          <p className="empty">This folder is empty.</p>
        ) : (
          <SunburstChart
            rootPath={path}
            nodes={nodes}
            totalBytes={totalBytes}
            scanning={rootScanning}
            onZoom={navigate}
            onReveal={reveal}
          />
        )}
      </div>
    </section>
  );
}

export default DiskMapView;
