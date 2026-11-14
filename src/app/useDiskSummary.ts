import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { errorText, runningInApp } from "../shared/format";

export type DiskSummary = {
  total: number;
  available: number;
  used: number;
};

/** Reads the capacity of the volume at `path` on mount, when it changes, and on `refresh`. */
export function useDiskSummary(path: string) {
  const [result, setResult] = useState<{ path: string; summary: DiskSummary } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [run, setRun] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `run` re-reads on refresh.
  useEffect(() => {
    if (!runningInApp()) {
      setError("Open Storage Saver to read this disk.");
      return;
    }
    let active = true;
    invoke<DiskSummary>("disk_summary", { path })
      .then((next) => {
        if (!active) return;
        setResult({ path, summary: next });
        setError(null);
      })
      .catch((reason: unknown) => {
        if (active) setError(errorText(reason));
      });
    return () => {
      active = false;
    };
  }, [path, run]);

  const refresh = useCallback(() => setRun((current) => current + 1), []);

  const summary = result?.path === path ? result.summary : null;
  return { summary, error, refresh };
}
