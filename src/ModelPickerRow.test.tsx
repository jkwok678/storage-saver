import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { modelChoices } from "./aiModels";
import { ModelPickerRow } from "./ModelPickerRow";

const choices = modelChoices("claude", [{ id: "opus", label: "Opus" }]);

type Overrides = Partial<Parameters<typeof ModelPickerRow>[0]>;

function renderRow(overrides: Overrides = {}) {
  const onPick = vi.fn();
  render(
    <ModelPickerRow
      choices={choices}
      pick={{ model: "opus", effort: "High" }}
      modelChoice={choices[1]}
      effortChoice={choices[1].efforts[3]}
      catalogError={undefined}
      connected
      disabled={false}
      onPick={onPick}
      {...overrides}
    />,
  );
  return onPick;
}

const select = (name: string) => screen.getByRole("combobox", { name }) as HTMLSelectElement;

describe("ModelPickerRow", () => {
  it("picks a model or an effort, keeping the other", () => {
    const onPick = renderRow();
    expect(select("Model").value).toBe("opus");
    expect(select("Effort").value).toBe("High");
    fireEvent.change(select("Model"), { target: { value: "default" } });
    expect(onPick).toHaveBeenCalledWith({ model: "default", effort: "High" });
    fireEvent.change(select("Effort"), { target: { value: "Low" } });
    expect(onPick).toHaveBeenCalledWith({ model: "opus", effort: "Low" });
  });

  it("explains an empty model list", () => {
    const empty = { choices: undefined, modelChoice: undefined, effortChoice: null };
    renderRow({ ...empty, connected: false });
    expect(select("Model").textContent).toBe("Default");
    expect(select("Model").disabled).toBe(true);
    expect(select("Effort").disabled).toBe(true);
  });

  it("shows loading and listing errors", () => {
    const empty = { choices: undefined, modelChoice: undefined, effortChoice: null };
    renderRow(empty);
    expect(screen.getByText("Loading…")).toBeTruthy();
    renderRow({ ...empty, catalogError: "Not signed in" });
    expect(screen.getByText("Could not list models")).toBeTruthy();
  });

  it("locks effort when a model has only one", () => {
    const single = { ...choices[1], efforts: [choices[1].efforts[0]] };
    renderRow({ modelChoice: single, effortChoice: single.efforts[0] });
    expect(select("Effort").disabled).toBe(true);
  });
});
