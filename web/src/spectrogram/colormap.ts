/** A compact inferno-like colormap (black → purple → orange → cream). */
const STOPS: Array<[number, number, number, number]> = [
  [0, 0, 0, 4],
  [0.25, 66, 10, 104],
  [0.5, 147, 38, 103],
  [0.75, 221, 81, 58],
  [0.9, 252, 165, 10],
  [1, 252, 255, 164],
];

export function inferno(t: number): [number, number, number] {
  const value = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0));
  for (let i = 1; i < STOPS.length; i += 1) {
    const [at, r, g, b] = STOPS[i];
    if (value <= at) {
      const [prevAt, pr, pg, pb] = STOPS[i - 1];
      const span = at - prevAt;
      const f = span > 0 ? (value - prevAt) / span : 0;
      return [
        Math.round(pr + (r - pr) * f),
        Math.round(pg + (g - pg) * f),
        Math.round(pb + (b - pb) * f),
      ];
    }
  }
  return [252, 255, 164];
}

export function infernoCss(t: number): string {
  const [r, g, b] = inferno(t);
  return `rgb(${r} ${g} ${b})`;
}

/** CSS gradient with the bright/high-dB end at the top and dark end at the bottom. */
export const INFERNO_CSS_GRADIENT = `linear-gradient(to bottom, ${[...STOPS]
  .reverse()
  .map(
    ([at, r, g, b]) =>
      `rgb(${r} ${g} ${b}) ${Math.round((1 - at) * 1_000) / 10}%`,
  )
  .join(", ")})`;
