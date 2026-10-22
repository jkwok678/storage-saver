import { describe, expect, it } from "vitest";
import { arcHsl, hueFromName } from "./color";

describe("hueFromName", () => {
  it("returns the same hue for the same name", () => {
    expect(hueFromName("Documents")).toBe(hueFromName("Documents"));
    expect(hueFromName("Library")).toBe(hueFromName("Library"));
  });

  it("returns different hues for different names", () => {
    expect(hueFromName("Documents")).not.toBe(hueFromName("Downloads"));
  });

  it("hue is in range 0..359", () => {
    for (const name of ["", "a", "very long folder name with spaces", "日本語フォルダ"]) {
      const h = hueFromName(name);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(360);
    }
  });
});

describe("arcHsl", () => {
  it("depth 0 uses sat 62% and light 54%", () => {
    expect(arcHsl(120, 0)).toBe("hsl(120, 62%, 54%)");
  });

  it("depth 1 reduces sat and increases light", () => {
    expect(arcHsl(120, 1)).toBe("hsl(120, 56%, 61%)");
  });

  it("depth 3 matches formula (sat 44%, light 75%)", () => {
    expect(arcHsl(0, 3)).toBe("hsl(0, 44%, 75%)");
  });

  it("clamps light at 100%", () => {
    // depth 7: light = 54 + 7*7 = 103 → clamped to 100
    const result = arcHsl(180, 7);
    expect(result).toContain("100%");
  });

  it("clamps sat at 0%", () => {
    // depth 11: sat = 62 - 6*11 = -4 → clamped to 0
    const result = arcHsl(180, 11);
    expect(result).toContain("0%");
  });

  it("saturation is monotonically non-increasing with depth", () => {
    const hue = hueFromName("Projects");
    const sats = [0, 1, 2, 3].map((d) => {
      const match = arcHsl(hue, d).match(/hsl\(\d+, (\d+)%/);
      return Number(match?.[1]);
    });
    for (let i = 1; i < sats.length; i++) {
      expect(sats[i]).toBeLessThanOrEqual(sats[i - 1]);
    }
  });

  it("lightness is monotonically non-decreasing with depth", () => {
    const hue = hueFromName("Projects");
    const lights = [0, 1, 2, 3].map((d) => {
      const match = arcHsl(hue, d).match(/, (\d+)%\)/);
      return Number(match?.[1]);
    });
    for (let i = 1; i < lights.length; i++) {
      expect(lights[i]).toBeGreaterThanOrEqual(lights[i - 1]);
    }
  });
});
