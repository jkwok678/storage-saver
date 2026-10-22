import { Channel, invoke } from "@tauri-apps/api/core";
import { type Dispatch, type SetStateAction, useEffect, useRef, useState } from "react";
import { errorText, runningInApp } from "../shared/format";
import { useStopOnUnmount } from "../shared/useStopOnUnmount";
import type { Entry } from "./entries";

type SizeUpdate = { path: string; bytes: number; error: string | null };

type ScanEvent =
  | { event: "started"; data: { entries: Entry[] } }
  | { event: "sized"; data: SizeUpdate }
  | { event: "done" };

export type ScanRequest = { path: string; refresh: boolean };

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
    const channel = new Channel<ScanEvent>();
    channel.onmessage = (message) => {
      if (!isCurrent()) return;
      if (message.event === "started") setEntries(message.data.entries);
      else if (message.event === "sized") sizes.add(message.data);
      else sizes.flushNow();
    };

    invoke("scan_directory", { path: request.path, refresh: request.refresh, onEvent: channel })
      .then(() => {
        if (isCurrent()) setScanning(false);
      })
      .catch((error: unknown) => {
        if (!isCurrent()) return;
        setScanError(errorText(error));
        setScanning(false);
      });

    return () => {
      active = false;
      sizes.cancel();
    };
  }, [request]);

  useStopOnUnmount("stop_scan");

  return { entries, scanning, scanError };
}
