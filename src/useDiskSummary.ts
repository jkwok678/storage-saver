import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { errorText, runningInApp } from "./format";

export type DiskSummary = {
  total: number;
  available: number;
  used: number;
};

/** Reads boot-disk capacity once on mount. */
export function useDiskSummary() {
  const [summary, setSummary] = useState<DiskSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!runningInApp()) {
      setError("Open Storage Saver to read this disk.");
      return;
    }
    let active = true;
    invoke<DiskSummary>("disk_summary")
      .then((next) => {
        if (active) setSummary(next);
      })
      .catch((reason: unknown) => {
        if (active) setError(errorText(reason));
      });
    return () => {
      active = false;
    };
  }, []);

  return { summary, error };
}
