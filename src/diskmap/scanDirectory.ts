import { Channel, invoke } from "@tauri-apps/api/core";
import type { Entry } from "../browse/entries";
import { errorText } from "../shared/format";

export type ScanRequest = { path: string; refresh: boolean };
export type SizeUpdate = { path: string; bytes: number; error: string | null };

export type ScanHandlers = {
  isCurrent: () => boolean;
  onStarted: (entries: Entry[]) => void;
  onSized: (update: SizeUpdate) => void;
  onDone: () => void;
  onError: (message: string) => void;
};

type ScanEvent =
  | { event: "started"; data: { entries: Entry[] } }
  | { event: "sized"; data: SizeUpdate }
  | { event: "done" };

/** Starts one scan_directory invoke. Returns a cancel that ignores later callbacks. */
export function startDirectoryScan(request: ScanRequest, handlers: ScanHandlers): () => void {
  let cancelled = false;
  let finished = false;
  const isCurrent = () => !cancelled && handlers.isCurrent();

  const finish = (error?: string) => {
    if (finished || !isCurrent()) return;
    finished = true;
    if (error !== undefined) handlers.onError(error);
    else handlers.onDone();
  };

  const channel = new Channel<ScanEvent>();
  channel.onmessage = (message) => {
    if (!isCurrent()) return;
    if (message.event === "started") handlers.onStarted(message.data.entries);
    else if (message.event === "sized") handlers.onSized(message.data);
    else finish();
  };

  invoke("scan_directory", { path: request.path, refresh: request.refresh, onEvent: channel }).then(
    () => finish(),
    (error: unknown) => finish(errorText(error)),
  );

  return () => {
    cancelled = true;
  };
}
