import type { AiProviderId } from "./aiProviders";

export type AiModel = {
  id: string;
  label: string;
};

/** What `query_ai` is sent for one effort of a model; null leaves the CLI's own default. */
export type EffortChoice = {
  label: string;
  model: string | null;
  effort: string | null;
};

export type ModelChoice = {
  key: string;
  label: string;
  efforts: EffortChoice[];
  /** The effort label to start at. */
  defaultEffort: string;
};

const EFFORTS = [
  { id: "low", label: "Low" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
  { id: "xhigh", label: "Extra high" },
  { id: "max", label: "Max" },
];

export const DEFAULT_LABEL = "Default";

function claudeEfforts(model: string | null): EffortChoice[] {
  return [
    { label: DEFAULT_LABEL, model, effort: null },
    ...EFFORTS.map((e) => ({ label: e.label, model, effort: e.id })),
  ];
}

const CURSOR_EFFORTS: Record<string, { rank: number; label: string }> = {
  none: { rank: 0, label: "None" },
  minimal: { rank: 1, label: "Minimal" },
  low: { rank: 2, label: "Low" },
  medium: { rank: 3, label: "Medium" },
  high: { rank: 4, label: "High" },
  xhigh: { rank: 5, label: "Extra high" },
  "extra-high": { rank: 5, label: "Extra high" },
  max: { rank: 6, label: "Max" },
};

// Cursor lists every effort as its own model, e.g. `claude-opus-5-5-high-fast`
// or `claude-4.6-opus-high-thinking`.
const CURSOR_VARIANT =
  /^(.+?)(?:-(none|minimal|low|medium|high|xhigh|extra-high|max))?(-thinking)?(-fast)?$/;
const EFFORT_WORDS = /\s*\b(Extra High|None|Minimal|Low|Medium|High|Max)\b/g;

type CursorGroup = {
  label: string;
  defaultEffort: string | null;
  variants: { rank: number; choice: EffortChoice }[];
};

/** Groups Cursor's per-effort models so effort can be picked on its own. */
function cursorChoices(models: AiModel[]): ModelChoice[] {
  const groups = new Map<string, CursorGroup>();
  for (const model of models) {
    const match = CURSOR_VARIANT.exec(model.id);
    if (!match) continue;
    const [, base, effortId, thinking, fast] = match;
    const key = `${base}${thinking ?? ""}${fast ?? ""}`;
    const effort = effortId ? CURSOR_EFFORTS[effortId] : null;
    const label = effort ? effort.label : DEFAULT_LABEL;
    const plainLabel = model.label.replace(EFFORT_WORDS, "").trim();
    const group = groups.get(key) ?? { label: plainLabel, defaultEffort: null, variants: [] };
    // Cursor leaves the effort out of the label of the variant it uses by default.
    if (plainLabel === model.label && !group.defaultEffort) {
      group.label = model.label;
      group.defaultEffort = label;
    }
    group.variants.push({
      rank: effort ? effort.rank : -1,
      choice: { label, model: model.id, effort: null },
    });
    groups.set(key, group);
  }
  return [...groups].map(([key, group]) => {
    const efforts = group.variants.sort((a, b) => a.rank - b.rank).map((v) => v.choice);
    return {
      key,
      label: group.label,
      efforts,
      defaultEffort:
        group.defaultEffort ?? (efforts.find((e) => e.label === "Medium") ?? efforts[0]).label,
    };
  });
}

export function modelChoices(provider: AiProviderId, models: AiModel[]): ModelChoice[] {
  if (provider === "cursor") return cursorChoices(models);
  return [
    {
      key: "default",
      label: DEFAULT_LABEL,
      efforts: claudeEfforts(null),
      defaultEffort: DEFAULT_LABEL,
    },
    ...models.map((m) => ({
      key: m.id,
      label: m.label,
      efforts: claudeEfforts(m.id),
      defaultEffort: DEFAULT_LABEL,
    })),
  ];
}

/** Keeps the effort the user had when a model offers it, else the model's default. */
export function pickEffort(choice: ModelChoice, wanted: string | null): EffortChoice {
  return (
    choice.efforts.find((e) => e.label === wanted) ??
    choice.efforts.find((e) => e.label === choice.defaultEffort) ??
    choice.efforts[0]
  );
}
