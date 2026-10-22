import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useState } from "react";
import { errorText, runningInApp } from "../shared/format";
import type { TrashOutcome } from "./aiTypes";

/** Reveal and move-to-Trash state for the recommended items. */
export function useTrash(onItemTrashed?: (path: string, freedBytes: number) => void) {
  // Track confirmation dialog & trashed states per path
  const [confirmingTrash, setConfirmingTrash] = useState<string | null>(null);
  const [trashingPath, setTrashingPath] = useState<string | null>(null);
  const [trashedPaths, setTrashedPaths] = useState<Map<string, TrashOutcome>>(new Map());
  const [trashError, setTrashError] = useState<{ path: string; message: string } | null>(null);

  const reveal = (path: string) => {
    if (!runningInApp()) return;
    revealItemInDir(path).catch((err: unknown) => {
      setTrashError({ path, message: errorText(err) });
    });
  };

  const moveToTrash = async (path: string) => {
    setTrashingPath(path);
    setTrashError(null);
    try {
      const result = await invoke<TrashOutcome>("move_to_trash", { path });
      setTrashedPaths((prev) => new Map(prev).set(path, result));
      setConfirmingTrash(null);
      if (onItemTrashed) {
        onItemTrashed(path, result.freedBytes);
      }
    } catch (err: unknown) {
      setTrashError({ path, message: errorText(err) });
    } finally {
      setTrashingPath(null);
    }
  };

  return {
    confirmingTrash,
    setConfirmingTrash,
    trashingPath,
    trashedPaths,
    trashError,
    clearTrashError: () => setTrashError(null),
    reveal,
    moveToTrash,
  };
}

export type TrashState = ReturnType<typeof useTrash>;
