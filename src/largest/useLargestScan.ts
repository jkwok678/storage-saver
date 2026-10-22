import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, isWithin, runningInApp } from "../shared/format";
import { useStopOnUnmount } from "../shared/useStopOnUnmount";

export type LargestFile = {
  name: string;
  path: string;
  bytes: number;
};

type LargestEvent =
  | { event: "batch"; data: { files: LargestFile[]; scanned: number } }
  | { event: "done"; data: { scanned: number } };

type LargestRequest = { path: string; limit: number };

const fullDiskScan = (): LargestRequest => ({ path: "/", limit: 100 });

/** Walks the whole disk, streaming the running top-N largest files. */
export function useLargestScan() {
  const [request, setRequest] = useState(fullDiskScan);
  const [files, setFiles] = useState<LargestFile[]>([]);
  const [scanned, setScanned] = useState(0);
  const [scanning, setScanning] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const scanGen = useRef(0);

  useEffect(() => {
    const gen = ++scanGen.current;
    const isCurrent = () => scanGen.current === gen;
    setFiles([]);
    setScanned(0);
    setScanning(true);
    setError(null);

    if (!runningInApp()) {
      setError("Open Storage Saver to read this disk.");
      setScanning(false);
      return;
    }

    const channel = new Channel<LargestEvent>();
    channel.onmessage = (message) => {
      if (!isCurrent()) return;
      setScanned(message.data.scanned);
      if (message.event === "batch") setFiles(message.data.files);
      else setScanning(false);
    };

    invoke("scan_largest_files", { ...request, onEvent: channel })
      .then(() => {
        if (isCurrent()) setScanning(false);
      })
      .catch((reason: unknown) => {
        if (!isCurrent()) return;
        setError(errorText(reason));
        setScanning(false);
      });

    return () => {
      scanGen.current += 1;
    };
  }, [request]);

  useStopOnUnmount("stop_largest_scan");

  const stop = () => {
    scanGen.current += 1;
    setScanning(false);
    if (runningInApp()) invoke("stop_largest_scan").catch(() => {});
  };

  const rescan = () => setRequest(fullDiskScan());

  // Drops a trashed item from the list instead of reading the whole disk again.
  const forget = useCallback((path: string) => {
    setFiles((current) => current.filter((file) => !isWithin(file.path, path)));
  }, []);

  return { files, scanned, scanning, error, stop, rescan, forget };
}
