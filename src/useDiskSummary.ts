import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { errorText, runningInApp } from "./format";

export type DiskSummary = {
  total: number;
  available: number;
  used: number;
};

/** Reads boot-disk capacity on mount and whenever `refresh` is called. */
export function useDiskSummary() {
  const [summary, setSummary] = useState<DiskSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [run, setRun] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `run` re-reads on refresh.
  useEffect(() => {
    if (!runningInApp()) {
      setError("Open Storage Saver to read this disk.");
      return;
    }
    let active = true;
    invoke<DiskSummary>("disk_summary")
      .then((next) => {
        if (!active) return;
        setSummary(next);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (active) setError(errorText(reason));
      });
    return () => {
      active = false;
    };
  }, [run]);

  const refresh = useCallback(() => setRun((current) => current + 1), []);

  return { summary, error, refresh };
}
