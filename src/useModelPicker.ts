import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { type AiModel, type ModelChoice, modelChoices, pickEffort } from "./aiModels";
import type { AiProviderId, AiProviderStatus } from "./aiProviders";
import { errorText, runningInApp } from "./format";

/** The model (by key) and effort (by label) picked for one provider. */
export type ModelPick = { model: string | null; effort: string | null };

const PICK_KEY = "aiAdvisor.model.";

function readPick(provider: AiProviderId): ModelPick {
  try {
    const stored = JSON.parse(localStorage.getItem(PICK_KEY + provider) ?? "null");
    if (stored && typeof stored === "object") {
      return {
        model: typeof stored.model === "string" ? stored.model : null,
        effort: typeof stored.effort === "string" ? stored.effort : null,
      };
    }
  } catch {
    // Storage can be missing or blocked; the pickers then start at the defaults.
  }
  return { model: null, effort: null };
}

function savePick(provider: AiProviderId, pick: ModelPick) {
  try {
    localStorage.setItem(PICK_KEY + provider, JSON.stringify(pick));
  } catch {
    // Not remembering the choice is fine.
  }
}

/** The model catalog, remembered pick and resolved choices for the selected provider. */
export function useModelPicker(selectedProvider: AiProviderId, activeProvider: AiProviderStatus) {
  const [catalogs, setCatalogs] = useState<Partial<Record<AiProviderId, ModelChoice[]>>>({});
  const [catalogErrors, setCatalogErrors] = useState<Partial<Record<AiProviderId, string>>>({});
  const [picks, setPicks] = useState<Record<AiProviderId, ModelPick>>(() => ({
    claude: readPick("claude"),
    cursor: readPick("cursor"),
  }));

  // Cursor's models depend on the account, so they are listed once signed in.
  const catalogRequested = useRef(new Set<AiProviderId>());
  useEffect(() => {
    const id = selectedProvider;
    if (!runningInApp() || !activeProvider.connected || catalogRequested.current.has(id)) return;
    catalogRequested.current.add(id);
    invoke<AiModel[]>("ai_models", { provider: id })
      .then((models) => {
        setCatalogs((prev) => ({ ...prev, [id]: modelChoices(id, models) }));
        setCatalogErrors((prev) => ({ ...prev, [id]: undefined }));
      })
      .catch((err: unknown) => {
        // Let a later visit try again, for example after signing in.
        catalogRequested.current.delete(id);
        setCatalogErrors((prev) => ({ ...prev, [id]: errorText(err) }));
      });
  }, [selectedProvider, activeProvider.connected]);

  const choices = catalogs[selectedProvider];
  const pick = picks[selectedProvider];
  const modelChoice = choices?.find((c) => c.key === pick.model) ?? choices?.[0];
  const effortChoice = modelChoice ? pickEffort(modelChoice, pick.effort) : null;

  const updatePick = (next: ModelPick) => {
    setPicks((prev) => ({ ...prev, [selectedProvider]: next }));
    savePick(selectedProvider, next);
  };

  return {
    choices,
    pick,
    modelChoice,
    effortChoice,
    catalogError: catalogErrors[selectedProvider],
    updatePick,
  };
}
