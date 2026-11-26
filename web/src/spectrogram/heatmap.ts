import { inferno } from "./colormap";
import { freqAtFraction, type Viewport } from "./geometry";
import type { DecodedGrid } from "./types";

/**
 * Map a dB value to the fixed palette domain. Values outside the selected
 * range clamp to the color at that range's edge instead of the palette ends,
 * so narrowing the color window only shows colors from that slice.
 */
export function colorStop(
  db: number,
  minDb: number,
  maxDb: number,
  domainMinDb = minDb,
  domainMaxDb = maxDb,
): number {
  const domainSpan = domainMaxDb > domainMinDb ? domainMaxDb - domainMinDb : 1;
  const range = maxDb > minDb ? maxDb - minDb : 1;
  const rawLow = (minDb - domainMinDb) / domainSpan;
  const rawHigh = (maxDb - domainMinDb) / domainSpan;
  const low = Math.min(1, Math.max(0, Math.min(rawLow, rawHigh)));
  const high = Math.min(1, Math.max(0, Math.max(rawLow, rawHigh)));
  const relative = Math.min(1, Math.max(0, (db - minDb) / range));
  return low + relative * (high - low);
}

/**
 * Build the RGBA pixels for a viewport. Every pixel row is mapped through the
 * viewport's log/linear frequency axis, so the main plot and the mini-map both
 * honour Zoom/Log/Linear and the configured frequency bounds. Each column is
 * sampled from the viewport's time range, and a row takes the loudest source
 * bin in its frequency band to keep narrow peaks visible.
 */
export function heatmapPixels(
  grid: DecodedGrid,
  view: Viewport,
  width: number,
  height: number,
  minDb: number,
  maxDb: number,
  domainMinDb = minDb,
  domainMaxDb = maxDb,
): Uint8ClampedArray {
  const w = Math.max(0, Math.floor(width));
  const h = Math.max(0, Math.floor(height));
  const pixels = new Uint8ClampedArray(w * h * 4);
  const [bgR, bgG, bgB] = inferno(0);
  for (let index = 0; index < pixels.length; index += 4) {
    pixels[index] = bgR;
    pixels[index + 1] = bgG;
    pixels[index + 2] = bgB;
    pixels[index + 3] = 255;
  }

  const { frames, bins, binHz, durationS, nyquist } = grid.meta;
  if (
    frames <= 0 ||
    bins <= 0 ||
    w === 0 ||
    h === 0 ||
    !(binHz > 0) ||
    !(nyquist > 0)
  ) {
    return pixels;
  }

  const range = maxDb > minDb ? maxDb - minDb : 1;
  const domainSpan =
    domainMaxDb > domainMinDb ? domainMaxDb - domainMinDb : 1;
  const rawLow = (minDb - domainMinDb) / domainSpan;
  const rawHigh = (maxDb - domainMinDb) / domainSpan;
  const lowStop = Math.min(1, Math.max(0, Math.min(rawLow, rawHigh)));
  const highStop = Math.min(1, Math.max(0, Math.max(rawLow, rawHigh)));
  const stopSpan = highStop - lowStop;
  const tSpan = view.t1 - view.t0;
  const frameByColumn = new Int32Array(w);
  for (let x = 0; x < w; x += 1) {
    if (frames === 1) {
      frameByColumn[x] = 0;
      continue;
    }
    if (durationS > 0 && tSpan > 0) {
      const time = view.t0 + ((x + 0.5) / w) * tSpan;
      if (time < 0 || time > durationS) {
        frameByColumn[x] = -1;
        continue;
      }
      frameByColumn[x] = Math.min(
        frames - 1,
        Math.max(0, Math.round((time / durationS) * (frames - 1))),
      );
    } else {
      // Degenerate duration: spread the available frames across the width.
      frameByColumn[x] = Math.min(
        frames - 1,
        Math.max(0, Math.round(((x + 0.5) / w) * (frames - 1))),
      );
    }
  }

  for (let y = 0; y < h; y += 1) {
    const fTop = freqAtFraction(1 - y / h, view);
    const fBottom = freqAtFraction(1 - (y + 1) / h, view);
    if (!Number.isFinite(fTop) || !Number.isFinite(fBottom)) {
      continue;
    }
    const highHz = Math.max(fTop, fBottom);
    const lowHz = Math.min(fTop, fBottom);
    if (highHz < 0 || lowHz > nyquist) {
      continue;
    }

    // Clamp the row band to the data, then take the loudest bin it covers.
    const binLo = Math.min(
      bins - 1,
      Math.max(0, Math.round(Math.max(0, lowHz) / binHz)),
    );
    const binHi = Math.min(
      bins - 1,
      Math.max(0, Math.round(Math.min(nyquist, highHz) / binHz)),
    );
    const firstBin = Math.min(binLo, binHi);
    const lastBin = Math.max(binLo, binHi);

    for (let x = 0; x < w; x += 1) {
      const frame = frameByColumn[x];
      if (frame < 0) {
        continue;
      }
      let db = -Infinity;
      for (let bin = firstBin; bin <= lastBin; bin += 1) {
        db = Math.max(db, grid.values[frame * bins + bin] / 100);
      }
      // Below the selected floor is genuinely out of range: leave the
      // background (black). Above the ceiling still clamps to the band's top
      // color so peaks stay bright.
      if (db < minDb) {
        continue;
      }
      const relative = Math.min(1, Math.max(0, (db - minDb) / range));
      const [r, g, b] = inferno(lowStop + relative * stopSpan);
      const index = (y * w + x) * 4;
      pixels[index] = r;
      pixels[index + 1] = g;
      pixels[index + 2] = b;
      pixels[index + 3] = 255;
    }
  }

  return pixels;
}
