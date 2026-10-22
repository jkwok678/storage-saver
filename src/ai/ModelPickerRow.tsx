import type { EffortChoice, ModelChoice } from "./aiModels";
import type { ModelPick } from "./useModelPicker";

type Props = {
  choices: ModelChoice[] | undefined;
  pick: ModelPick;
  modelChoice: ModelChoice | undefined;
  effortChoice: EffortChoice | null;
  catalogError: string | undefined;
  connected: boolean;
  disabled: boolean;
  onPick: (next: ModelPick) => void;
};

export function ModelPickerRow({
  choices,
  pick,
  modelChoice,
  effortChoice,
  catalogError,
  connected,
  disabled,
  onPick,
}: Props) {
  return (
    <div className="ai-model-row">
      <label className="ai-model-field">
        <span>Model</span>
        <select
          value={modelChoice?.key ?? ""}
          onChange={(e) => onPick({ model: e.target.value, effort: pick.effort })}
          disabled={disabled || !choices}
          title={catalogError}
        >
          {choices ? (
            choices.map((choice) => (
              <option key={choice.key} value={choice.key}>
                {choice.label}
              </option>
            ))
          ) : (
            <option value="">
              {catalogError ? "Could not list models" : connected ? "Loading…" : "Default"}
            </option>
          )}
        </select>
      </label>
      <label className="ai-model-field">
        <span>Effort</span>
        <select
          value={effortChoice?.label ?? ""}
          onChange={(e) => onPick({ model: pick.model, effort: e.target.value })}
          disabled={disabled || !modelChoice || modelChoice.efforts.length < 2}
        >
          {modelChoice ? (
            modelChoice.efforts.map((effort) => (
              <option key={effort.label} value={effort.label}>
                {effort.label}
              </option>
            ))
          ) : (
            <option value="">Default</option>
          )}
        </select>
      </label>
    </div>
  );
}
