import { describe, expect, it } from "vitest";
import { annularPath, polar } from "./geometry";

describe("polar", () => {
  it("places 0 at 12 o'clock", () => {
    const p = polar(50, 50, 10, 0);
    expect(p.x).toBeCloseTo(50);
    expect(p.y).toBeCloseTo(40); // r above center
  });

  it("places π/2 at 3 o'clock", () => {
    const p = polar(50, 50, 10, Math.PI / 2);
    expect(p.x).toBeCloseTo(60);
    expect(p.y).toBeCloseTo(50);
  });

  it("places π at 6 o'clock", () => {
    const p = polar(50, 50, 10, Math.PI);
    expect(p.x).toBeCloseTo(50);
    expect(p.y).toBeCloseTo(60);
  });
});

describe("annularPath", () => {
  it("returns empty string when a0 === a1", () => {
    expect(annularPath(50, 50, 10, 20, 1, 1)).toBe("");
  });

  it("returns a non-empty path for a 90° sector", () => {
    const d = annularPath(50, 50, 10, 20, 0, Math.PI / 2);
    expect(d).toMatch(/^M /);
    expect(d).toContain("A");
    expect(d).toContain("Z");
  });

  it("uses large-arc flag 0 for sweep < π", () => {
    const d = annularPath(50, 50, 10, 20, 0, Math.PI / 2);
    // large-arc-flag is 0 for small sweep
    expect(d).toContain("0 1");
  });

  it("uses large-arc flag 1 for sweep > π", () => {
    const d = annularPath(50, 50, 10, 20, 0, Math.PI * 1.5);
    expect(d).toContain("1 1");
  });

  it("splits a full circle (2π) into two halves to avoid SVG degeneracy", () => {
    const d = annularPath(50, 50, 10, 20, 0, Math.PI * 2);
    // Two M commands (two half-circles joined)
    const mCount = (d.match(/M /g) ?? []).length;
    expect(mCount).toBe(2);
  });

  it("splits near-full sweeps (≥ 2π − ε)", () => {
    const d = annularPath(50, 50, 10, 20, 0, Math.PI * 2 - 1e-7);
    const mCount = (d.match(/M /g) ?? []).length;
    expect(mCount).toBe(2);
  });
});
