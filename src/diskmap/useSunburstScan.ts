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

export function useSunburstScan(request: ScanRequest): SunburstScan {
  const [listings, setListings] = useState<ReadonlyMap<string, Entry[]>>(new Map());
  const [scanning, setScanning] = useState(false);
  const [rootScanning, setRootScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const listingsRef = useRef<Map<string, Entry[]>>(new Map());
  const genRef = useRef(0);
  const queueRef = useRef<QueueItem[]>([]);
  const inFlightRef = useRef(false);
  const rafRef = useRef<number | null>(null);
  const sizeBufferRef = useRef<Map<string, Map<string, SizeUpdate>>>(new Map());
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

    const gen = ++genRef.current;
    const isCurrent = () => genRef.current === gen;
    let cancelScan = () => {};

    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    sizeBufferRef.current.clear();
    inFlightRef.current = false;

    const prevRoot = listingsRef.current.get(request.path);
    listingsRef.current = new Map(prevRoot ? [[request.path, prevRoot]] : []);

    queueRef.current = [
      {
        path: request.path,
        depth: 0,
        refresh: request.refresh || useRefresh,
        parentFraction: 1,
      },
    ];

    setScanning(true);
    setRootScanning(true);
    setScanError(null);
    setListings(new Map(listingsRef.current));

    const flushBuffer = () => {
      for (const [listingPath, updates] of sizeBufferRef.current) {
        if (updates.size === 0) continue;
        const entries = listingsRef.current.get(listingPath);
        if (!entries) continue;
        listingsRef.current.set(
          listingPath,
          entries.map((entry) => {
            const update = updates.get(entry.path);
            return update ? { ...entry, bytes: update.bytes, error: update.error } : entry;
          }),
        );
        updates.clear();
      }
    };

    const processNext = (): void => {
      if (inFlightRef.current || queueRef.current.length === 0 || !isCurrent()) return;

      const item = queueRef.current.shift();
      if (!item) return;
      inFlightRef.current = true;
      sizeBufferRef.current.set(item.path, new Map());

      const handlers: ScanHandlers = {
        isCurrent,
        onStarted(entries) {
          if (!isCurrent()) return;
          listingsRef.current.set(item.path, entries);
          setListings(new Map(listingsRef.current));
        },
        onSized(update) {
          if (!isCurrent()) return;
          sizeBufferRef.current.get(item.path)?.set(update.path, update);
          if (rafRef.current === null) {
            rafRef.current = requestAnimationFrame(() => {
              rafRef.current = null;
              if (!isCurrent()) return;
              flushBuffer();
              setListings(new Map(listingsRef.current));
            });
          }
        },
        onDone() {
          if (!isCurrent()) return;
          if (rafRef.current !== null) {
            cancelAnimationFrame(rafRef.current);
            rafRef.current = null;
          }
          flushBuffer();
          inFlightRef.current = false;
          if (item.depth === 0) setRootScanning(false);

          const entries = listingsRef.current.get(item.path) ?? [];
          if (item.depth + 1 < MAX_RINGS) {
            const total = entries.reduce((sum, entry) => sum + (entry.bytes ?? 0), 0);
            for (const entry of entries) {
              if (!entry.isDir || entry.error || entry.bytes === null || entry.bytes === 0)
                continue;
              const childFraction = total > 0 ? (entry.bytes / total) * item.parentFraction : 0;
              if (childFraction >= MIN_EXPAND_TURN) {
                queueRef.current.push({
                  path: entry.path,
                  depth: item.depth + 1,
                  refresh: false,
                  parentFraction: childFraction,
                });
              }
            }
          }

          setListings(new Map(listingsRef.current));
          if (queueRef.current.length === 0) setScanning(false);
          else processNext();
        },
        onError(message) {
          if (!isCurrent()) return;
          inFlightRef.current = false;
          if (item.depth === 0) {
            if (rafRef.current !== null) {
              cancelAnimationFrame(rafRef.current);
              rafRef.current = null;
            }
            setScanError(message);
            setScanning(false);
            setRootScanning(false);
            return;
          }
          setListings(new Map(listingsRef.current));
          if (queueRef.current.length === 0) setScanning(false);
          else processNext();
        },
      };

      cancelScan = startDirectoryScan({ path: item.path, refresh: item.refresh }, handlers);
    };

    processNext();

    return () => {
      genRef.current += 1;
      cancelScan();
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [request, tick]);

  useStopOnUnmount("stop_scan");

  const rootEntries = listings.get(request.path) ?? [];
  return { listings, rootEntries, scanning, rootScanning, scanError, refresh };
}
