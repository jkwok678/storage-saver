import { useCallback, useEffect, useRef, useState } from "react";
import type { Entry } from "../browse/entries";
import { runningInApp } from "../shared/format";
import { useStopOnUnmount } from "../shared/useStopOnUnmount";
import { MAX_RINGS, MIN_EXPAND_TURN } from "./layout";
import {
  type ScanHandlers,
  type ScanRequest,
  type SizeUpdate,
  startDirectoryScan,
} from "./scanDirectory";

export type SunburstScan = {
  listings: ReadonlyMap<string, Entry[]>;
  rootEntries: Entry[];
  scanning: boolean;
  rootScanning: boolean;
  scanError: string | null;
  refresh: () => void;
};

type QueueItem = {
  path: string;
  depth: number;
  refresh: boolean;
  parentFraction: number;
};

type SizeBuffer = Map<string, Map<string, SizeUpdate>>;

function cancelFrame(rafRef: { current: number | null }): void {
  if (rafRef.current === null) return;
  cancelAnimationFrame(rafRef.current);
  rafRef.current = null;
}

function scheduleFrame(rafRef: { current: number | null }, onFrame: () => void): void {
  if (rafRef.current !== null) return;
  rafRef.current = requestAnimationFrame(() => {
    rafRef.current = null;
    onFrame();
  });
}

function applySizeUpdates(listings: Map<string, Entry[]>, buffer: SizeBuffer): void {
  for (const [listingPath, updates] of buffer) {
    if (updates.size === 0) continue;
    const entries = listings.get(listingPath);
    if (!entries) continue;
    listings.set(
      listingPath,
      entries.map((entry) => {
        const update = updates.get(entry.path);
        return update ? { ...entry, bytes: update.bytes, error: update.error } : entry;
      }),
    );
    updates.clear();
  }
}

/** Child folders of a finished listing that are wide enough to show a ring. */
function childQueueItems(item: QueueItem, entries: Entry[]): QueueItem[] {
  if (item.depth + 1 >= MAX_RINGS) return [];
  const total = entries.reduce((sum, entry) => sum + (entry.bytes ?? 0), 0);
  if (total === 0) return [];
  const children: QueueItem[] = [];
  for (const entry of entries) {
    if (!entry.isDir || entry.error || !entry.bytes) continue;
    const parentFraction = (entry.bytes / total) * item.parentFraction;
    if (parentFraction >= MIN_EXPAND_TURN) {
      children.push({ path: entry.path, depth: item.depth + 1, refresh: false, parentFraction });
    }
  }
  return children;
}

type RunCallbacks = {
  onListings: (listings: Map<string, Entry[]>) => void;
  onRootDone: () => void;
  onFinished: () => void;
  onRootError: (message: string) => void;
};

/**
 * Scans `root`, then walks breadth-first into folders wide enough to show a
 * ring, one listing at a time. Mutates `listings` in place and publishes
 * copies through `cb`. Returns a cancel function.
 */
function runSunburstScan(
  root: QueueItem,
  listings: Map<string, Entry[]>,
  cb: RunCallbacks,
): () => void {
  let active = true;
  let inFlight = false;
  let cancelScan = () => {};
  const isCurrent = () => active;
  const queue: QueueItem[] = [root];
  const sizeBuffer: SizeBuffer = new Map();
  const raf: { current: number | null } = { current: null };

  const publish = () => cb.onListings(new Map(listings));
  const flush = () => applySizeUpdates(listings, sizeBuffer);

  const publishSizes = () => {
    if (!active) return;
    flush();
    publish();
  };

  const advance = () => {
    publish();
    if (queue.length === 0) cb.onFinished();
    else processNext();
  };

  const processNext = (): void => {
    if (inFlight || !active) return;
    const item = queue.shift();
    if (!item) return;
    inFlight = true;
    sizeBuffer.set(item.path, new Map());

    const handlers: ScanHandlers = {
      isCurrent,
      onStarted(entries) {
        if (!active) return;
        listings.set(item.path, entries);
        publish();
      },
      onSized(update) {
        if (!active) return;
        sizeBuffer.get(item.path)?.set(update.path, update);
        scheduleFrame(raf, publishSizes);
      },
      onDone() {
        if (!active) return;
        cancelFrame(raf);
        flush();
        inFlight = false;
        if (item.depth === 0) cb.onRootDone();
        queue.push(...childQueueItems(item, listings.get(item.path) ?? []));
        advance();
      },
      onError(message) {
        if (!active) return;
        inFlight = false;
        if (item.depth !== 0) {
          advance();
          return;
        }
        cancelFrame(raf);
        cb.onRootError(message);
      },
    };

    cancelScan = startDirectoryScan({ path: item.path, refresh: item.refresh }, handlers);
  };

  processNext();

  return () => {
    active = false;
    cancelScan();
    cancelFrame(raf);
  };
}

export function useSunburstScan(request: ScanRequest): SunburstScan {
  const [listings, setListings] = useState<ReadonlyMap<string, Entry[]>>(new Map());
  const [scanning, setScanning] = useState(false);
  const [rootScanning, setRootScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const listingsRef = useRef<Map<string, Entry[]>>(new Map());
  const refreshRef = useRef(false);

  const refresh = useCallback(() => {
    refreshRef.current = true;
    setTick((t) => t + 1);
  }, []);

  // tick is bumped by refresh() so the same request object scans again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh retrigger
  useEffect(() => {
    if (!runningInApp()) {
      setScanError("Open Storage Saver to read this disk.");
      setScanning(false);
      setRootScanning(false);
      return;
    }

    const useRefresh = refreshRef.current;
    refreshRef.current = false;

    const prevRoot = listingsRef.current.get(request.path);
    listingsRef.current = new Map(prevRoot ? [[request.path, prevRoot]] : []);

    setScanning(true);
    setRootScanning(true);
    setScanError(null);
    setListings(new Map(listingsRef.current));

    return runSunburstScan(
      { path: request.path, depth: 0, refresh: request.refresh || useRefresh, parentFraction: 1 },
      listingsRef.current,
      {
        onListings: setListings,
        onRootDone: () => setRootScanning(false),
        onFinished: () => setScanning(false),
        onRootError: (message) => {
          setScanError(message);
          setScanning(false);
          setRootScanning(false);
        },
      },
    );
  }, [request, tick]);

  useStopOnUnmount("stop_scan");

  const rootEntries = listings.get(request.path) ?? [];
  return { listings, rootEntries, scanning, rootScanning, scanError, refresh };
}
