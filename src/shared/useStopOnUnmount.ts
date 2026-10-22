import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";
import { runningInApp } from "./format";

/** Invokes a backend stop command once the component has unmounted for good. */
export function useStopOnUnmount(command: string) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      // Deferred so a StrictMode remount does not cancel the scan it just started.
      queueMicrotask(() => {
        if (!mounted.current && runningInApp()) invoke(command).catch(() => {});
      });
    };
  }, [command]);
}
