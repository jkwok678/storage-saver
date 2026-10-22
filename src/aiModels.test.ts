import { describe, expect, it } from "vitest";
import { modelChoices, pickEffort } from "./aiModels";

describe("modelChoices for Claude Code", () => {
  it("offers the CLI default first and every effort per model", () => {
    const choices = modelChoices("claude", [{ id: "opus", label: "Opus" }]);
    expect(choices.map((c) => c.key)).toEqual(["default", "opus"]);
    expect(choices[1].efforts.map((e) => e.label)).toEqual([
      "Default",
      "Low",
      "Medium",
      "High",
      "Extra high",
      "Max",
    ]);
    expect(choices[1].efforts[2]).toEqual({ label: "Medium", model: "opus", effort: "medium" });
    expect(choices[0].efforts[0]).toEqual({ label: "Default", model: null, effort: null });
  });
});

describe("modelChoices for Cursor", () => {
  it("groups per-effort models and starts at the unlabelled variant", () => {
    const [choice] = modelChoices("cursor", [
      { id: "gpt-5-high", label: "GPT-5 High" },
      { id: "gpt-5", label: "GPT-5" },
      { id: "gpt-5-low", label: "GPT-5 Low" },
    ]);
    expect(choice.key).toBe("gpt-5");
    expect(choice.label).toBe("GPT-5");
    expect(choice.defaultEffort).toBe("Default");
    expect(choice.efforts.map((e) => [e.label, e.model])).toEqual([
      ["Default", "gpt-5"],
      ["Low", "gpt-5-low"],
      ["High", "gpt-5-high"],
    ]);
    expect(choice.efforts.every((e) => e.effort === null)).toBe(true);
  });

  it("keeps thinking and fast variants as their own models", () => {
    const choices = modelChoices("cursor", [
      { id: "claude-opus-high-thinking", label: "Opus High Thinking" },
      { id: "claude-opus-max-thinking", label: "Opus Max Thinking" },
      { id: "claude-opus-high-fast", label: "Opus High Fast" },
    ]);
    expect(choices.map((c) => c.key)).toEqual(["claude-opus-thinking", "claude-opus-fast"]);
    expect(choices[0].label).toBe("Opus Thinking");
    expect(choices[0].efforts.map((e) => e.label)).toEqual(["High", "Max"]);
  });

  it("starts at Medium, else the lowest effort, when every variant names one", () => {
    const [withMedium, withoutMedium] = modelChoices("cursor", [
      { id: "a-low", label: "A Low" },
      { id: "a-medium", label: "A Medium" },
      { id: "b-high", label: "B High" },
      { id: "b-extra-high", label: "B Extra High" },
    ]);
    expect(withMedium.defaultEffort).toBe("Medium");
    expect(withoutMedium.defaultEffort).toBe("High");
    expect(withoutMedium.efforts.map((e) => e.label)).toEqual(["High", "Extra high"]);
  });
});

describe("pickEffort", () => {
  const [, opus] = modelChoices("claude", [{ id: "opus", label: "Opus" }]);

  it("keeps the wanted effort when the model offers it", () => {
    expect(pickEffort(opus, "High").effort).toBe("high");
  });

  it("falls back to the model's default effort", () => {
    expect(pickEffort(opus, "Turbo").label).toBe("Default");
    expect(pickEffort({ ...opus, defaultEffort: "Gone" }, null).label).toBe("Default");
  });
});
