/** Deterministic hue 0–359 from a folder name; 31-hash of UTF-16 code units. */
export function hueFromName(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) {
    h = (Math.imul(31, h) + name.charCodeAt(i)) | 0;
  }
  return ((h % 360) + 360) % 360;
}

/** HSL fill for a folder arc at the given depth.
 *  depth 0: sat 62%, light 54%; each extra depth: −6 sat, +7 light (clamped).
 *  Dark-mode adjustment is handled in CSS via filter, not in JS. */
export function arcHsl(hue: number, depth: number): string {
  const sat = Math.max(0, 62 - 6 * depth);
  const light = Math.min(100, 54 + 7 * depth);
  return `hsl(${hue}, ${sat}%, ${light}%)`;
}
