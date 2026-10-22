import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import { errorText, runningInApp } from "../shared/format";

export type AiProviderId = "claude" | "cursor";

export type AiProviderStatus = {
  id: AiProviderId;
  name: string;
  installed: boolean;
  connected: boolean;
  account?: string | null;
  detail?: string | null;
};

export const PROVIDER_IDS: AiProviderId[] = ["claude", "cursor"];

export function providerName(id: AiProviderId) {
  return id === "claude" ? "Claude Code" : "Cursor";
}

/** Sign-in state for the AI CLIs, shared by Settings and the AI Advisor. */
export function useAiProviders() {
  const [providers, setProviders] = useState<AiProviderStatus[] | null>(null);
  const [busy, setBusy] = useState<AiProviderId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const requested = useRef(false);

  const refresh = useCallback(() => {
    if (!runningInApp()) return;
    requested.current = true;
    setError(null);
    invoke<AiProviderStatus[]>("ai_providers_status")
      .then(setProviders)
      .catch((err: unknown) => {
        setProviders((prev) => prev ?? []);
        setError(errorText(err));
      });
  }, []);

  /** Checks the CLIs once; later checks only happen on refresh. */
  const load = useCallback(() => {
    if (!requested.current) refresh();
  }, [refresh]);

  const upsert = (updated: AiProviderStatus) => {
    setProviders((prev) => {
      const list = prev ?? [];
      return list.some((p) => p.id === updated.id)
        ? list.map((p) => (p.id === updated.id ? updated : p))
        : [...list, updated];
    });
  };

  const run = async (command: string, id: AiProviderId) => {
    if (!runningInApp()) return;
    setBusy(id);
    setError(null);
    try {
      upsert(await invoke<AiProviderStatus>(command, { provider: id }));
    } catch (err: unknown) {
      setError(errorText(err));
    } finally {
      setBusy(null);
    }
  };

  return {
    providers,
    /** True until the first status check comes back. */
    checking: runningInApp() && providers === null,
    busy,
    error,
    load,
    refresh,
    connect: (id: AiProviderId) => run("connect_ai_provider", id),
    disconnect: (id: AiProviderId) => run("disconnect_ai_provider", id),
  };
}

export type AiProviders = ReturnType<typeof useAiProviders>;

/** Falls back to a placeholder while the CLIs are still being checked. */
export function findProvider(ai: AiProviders, id: AiProviderId): AiProviderStatus {
  return (
    ai.providers?.find((p) => p.id === id) ?? {
      id,
      name: providerName(id),
      installed: !ai.checking,
      connected: false,
    }
  );
}

export function providerStatusText(ai: AiProviders, provider: AiProviderStatus) {
  if (!runningInApp()) return "Open Storage Saver to connect accounts";
  if (ai.checking) return "Checking connection…";
  if (provider.connected) return provider.account || provider.detail || "Connected";
  if (provider.installed) return "Not signed in";
  return `${provider.name} CLI not found`;
}

export function providerDotClass(ai: AiProviders, provider: AiProviderStatus) {
  if (provider.connected) return "connected";
  return provider.installed || ai.checking ? "disconnected" : "missing";
}
