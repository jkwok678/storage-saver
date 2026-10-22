/** Polar to cartesian: angle 0 at 12 o'clock, clockwise. */
export function polar(cx: number, cy: number, r: number, a: number): { x: number; y: number } {
  return { x: cx + r * Math.sin(a), y: cy - r * Math.cos(a) };
}

/** Annular sector SVG `d` string.
 *  Returns "" when a0 === a1.
 *  Splits sweeps ≥ 2π − ε into two 180° sectors to avoid SVG full-circle degeneracy. */
export function annularPath(
  cx: number,
  cy: number,
  r0: number,
  r1: number,
  a0: number,
  a1: number,
): string {
  if (a0 === a1) return "";

  const sweep = a1 - a0;
  if (sweep >= Math.PI * 2 - 1e-6) {
    const mid = a0 + Math.PI;
    return `${annularPath(cx, cy, r0, r1, a0, mid)} ${annularPath(cx, cy, r0, r1, mid, a1)}`;
  }

  const p0 = polar(cx, cy, r1, a0);
  const p1 = polar(cx, cy, r1, a1);
  const p2 = polar(cx, cy, r0, a1);
  const p3 = polar(cx, cy, r0, a0);
  const large = sweep > Math.PI ? 1 : 0;
  const f = (n: number) => n.toFixed(4);

  return [
    `M ${f(p0.x)} ${f(p0.y)}`,
    `A ${f(r1)} ${f(r1)} 0 ${large} 1 ${f(p1.x)} ${f(p1.y)}`,
    `L ${f(p2.x)} ${f(p2.y)}`,
    `A ${f(r0)} ${f(r0)} 0 ${large} 0 ${f(p3.x)} ${f(p3.y)}`,
    "Z",
  ].join(" ");
}
