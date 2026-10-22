import { type Dispatch, type SetStateAction, useEffect, useRef, useState } from "react";
import { type ScanRequest, type SizeUpdate, startDirectoryScan } from "../diskmap/scanDirectory";
import { runningInApp } from "../shared/format";
import { useStopOnUnmount } from "../shared/useStopOnUnmount";
import type { Entry } from "./entries";

export type { ScanRequest };

/** Coalesces per-entry size events into at most one state update per animation frame. */
function batchSizeUpdates(isCurrent: () => boolean, setEntries: Dispatch<SetStateAction<Entry[]>>) {
  let frame: number | null = null;
  const updates = new Map<string, SizeUpdate>();

  const flush = () => {
    frame = null;
    if (!isCurrent() || updates.size === 0) {
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

  const cancel = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
  };

  return {
    add(update: SizeUpdate) {
      updates.set(update.path, update);
      if (frame === null) frame = requestAnimationFrame(flush);
    },
    flushNow() {
      cancel();
      flush();
    },
    cancel,
  };
}

/** Lists a folder, then streams in each entry's size as the backend measures it. */
export function useDirectoryScan(request: ScanRequest) {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [scanning, setScanning] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);
  const scanGen = useRef(0);

  useEffect(() => {
    const gen = ++scanGen.current;
    let active = true;
    const isCurrent = () => active && scanGen.current === gen;

    setEntries([]);
    setScanning(true);
    setScanError(null);

    if (!runningInApp()) {
      setScanError("Open Storage Saver to read this disk.");
      setScanning(false);
      return;
    }

    const sizes = batchSizeUpdates(isCurrent, setEntries);
    const cancel = startDirectoryScan(request, {
      isCurrent,
      onStarted: (e) => setEntries(e),
      onSized: (update) => sizes.add(update),
      onDone: () => {
        sizes.flushNow();
        setScanning(false);
      },
      onError: (message) => {
        sizes.cancel();
        setScanError(message);
        setScanning(false);
      },
    });

    return () => {
      active = false;
      cancel();
      sizes.cancel();
    };
  }, [request]);

  useStopOnUnmount("stop_scan");

  return { entries, scanning, scanError };
}
