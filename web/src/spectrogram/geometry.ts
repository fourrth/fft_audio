/** Pure viewport / colorbar / minimap maths for the canvas displays. */

export const LOG_MIN_HZ = 10;

/** Visual-only overscan allowed around the analysed data (5%). */
export const VIEW_MARGIN = 0.05;

export interface Viewport {
  t0: number;
  t1: number;
  f0: number;
  f1: number;
  logFreq: boolean;
}

/** Hard frequency limits for a viewport (view data can never leave them). */
export interface FrequencyBounds {
  minHz: number;
  maxHz: number;
}

export interface MinimapRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function safeTop(topHz: number): number {
  return Number.isFinite(topHz) && topHz > 0 ? topHz : 1;
}

/**
 * Effective log floor for a viewport. Keeps a strictly positive log span even
 * when the requested top is below `LOG_MIN_HZ` (for example on very low-rate
 * data), where a plain `max(f0, LOG_MIN_HZ)` would collapse the axis.
 */
export function logFloor(
  f0: number,
  f1: number,
  logMin = LOG_MIN_HZ,
): number {
  const top = Number.isFinite(f1) && f1 > 0 ? f1 : 1;
  const minimum =
    Number.isFinite(logMin) && logMin > 0 ? logMin : LOG_MIN_HZ;
  const requested = Number.isFinite(f0) ? Math.max(f0, minimum) : minimum;
  // Keep the requested floor for normal/narrow ranges; only fall back to a
  // synthetic floor when the requested floor would collapse the axis.
  return requested < top ? requested : top * 0.5;
}

function frequencyFloor(minHz: number, topHz: number, logFreq: boolean): number {
  const requested = Number.isFinite(minHz) ? minHz : 0;
  if (logFreq) {
    const requestedLogFloor = Math.max(requested, LOG_MIN_HZ);
    return requestedLogFloor < topHz ? requestedLogFloor : topHz * 0.5;
  }
  return Math.min(Math.max(requested, 0), topHz);
}

function boundedLimits(bounds: FrequencyBounds, logFreq: boolean): {
  floor: number;
  top: number;
} {
  const top = safeTop(bounds.maxHz);
  return { floor: frequencyFloor(bounds.minHz, top, logFreq), top };
}

export function fitView(
  durationS: number,
  nyquist: number,
  logFreq: boolean,
  minHz = 0,
): Viewport {
  const duration = Number.isFinite(durationS) && durationS > 0 ? durationS : 1;
  const top = safeTop(nyquist);
  const floor = frequencyFloor(minHz, top, logFreq);
  return {
    t0: 0,
    t1: duration,
    f0: floor,
    f1: top,
    logFreq,
  };
}

export function clampView(
  view: Viewport,
  durationS: number,
  nyquist: number,
  bounds?: FrequencyBounds,
): Viewport {
  const duration = Number.isFinite(durationS) && durationS > 0 ? durationS : 1;
  const top = safeTop(nyquist);
  const marginT = duration * VIEW_MARGIN;
  const minT = -marginT;
  const maxT = duration + marginT;
  const tSpan = Math.min(
    Math.max(view.t1 - view.t0, 1e-6),
    maxT - minT,
  );
  const t0 = Math.min(Math.max(view.t0, minT), maxT - tSpan);

  let f0: number;
  let f1: number;
  if (bounds) {
    // Hard bounds: no vertical overscan, shift a too-large span back inside.
    const limits = boundedLimits(bounds, view.logFreq);
    const allowedSpan = Math.max(0, limits.top - limits.floor);
    let fSpan = Math.max(view.f1 - view.f0, limits.top * 1e-6);
    fSpan = Math.min(fSpan, allowedSpan);
    f0 = Math.min(
      Math.max(Number.isFinite(view.f0) ? view.f0 : limits.floor, limits.floor),
      limits.top - fSpan,
    );
    f1 = f0 + fSpan;
  } else {
    const logAxisFloor = Math.max(LOG_MIN_HZ * (1 - VIEW_MARGIN), 1);
    const fMin = view.logFreq
      ? Math.min(logAxisFloor, top * 0.5)
      : -top * VIEW_MARGIN;
    const fMax = top * (1 + VIEW_MARGIN);
    let fSpan = Math.max(view.f1 - view.f0, top * 1e-6);
    fSpan = Math.min(fSpan, fMax - fMin);
    f0 = Math.min(Math.max(view.f0, fMin), fMax - fSpan);
    f1 = f0 + fSpan;
  }
  return {
    t0,
    t1: t0 + tSpan,
    f0,
    f1,
    logFreq: view.logFreq,
  };
}

export function withLogFreq(
  view: Viewport,
  logFreq: boolean,
  top: number,
  minHz = 0,
): Viewport {
  const safe = safeTop(top);
  const floor = frequencyFloor(minHz, safe, logFreq);
  return {
    ...view,
    logFreq,
    f0: floor,
    f1: safe,
  };
}

/** Shift a view by data-space deltas, then clamp it to the available data. */
export function panView(
  view: Viewport,
  dt: number,
  df: number,
  durationS: number,
  nyquist: number,
  bounds?: FrequencyBounds,
): Viewport {
  return clampView(
    {
      ...view,
      t0: view.t0 + dt,
      t1: view.t1 + dt,
      f0: view.f0 + df,
      f1: view.f1 + df,
    },
    durationS,
    nyquist,
    bounds,
  );
}

/**
 * Zoom around `(anchorT, anchorF)`. `factor < 1` zooms in and `factor > 1`
 * zooms out. The log axis scales geometrically so the cursor frequency stays
 * put; the linear axis scales in Hz.
 */
export function zoomView(
  view: Viewport,
  factor: number,
  anchorT: number,
  anchorF: number,
  durationS: number,
  nyquist: number,
  logFreq: boolean,
  bounds?: FrequencyBounds,
): Viewport {
  const scale = Number.isFinite(factor) && factor > 0 ? factor : 1;
  const base: Viewport = { ...view, logFreq };
  const anchorTime = Math.min(base.t1, Math.max(base.t0, anchorT));
  const t0 = anchorTime - (anchorTime - base.t0) * scale;
  const t1 = anchorTime + (base.t1 - anchorTime) * scale;

  let f0: number;
  let f1: number;
  if (base.logFreq) {
    const lo = logFloor(base.f0, base.f1);
    const anchor = Math.min(base.f1, Math.max(lo, anchorF));
    const logAnchor = Math.log(anchor);
    f0 = Math.exp(logAnchor + (Math.log(lo) - logAnchor) * scale);
    f1 = Math.exp(logAnchor + (Math.log(base.f1) - logAnchor) * scale);
  } else {
    const anchor = Math.min(base.f1, Math.max(base.f0, anchorF));
    f0 = anchor - (anchor - base.f0) * scale;
    f1 = anchor + (base.f1 - anchor) * scale;
  }

  return clampView({ ...base, t0, t1, f0, f1 }, durationS, nyquist, bounds);
}

/**
 * Convert a pixel rectangle inside the plot into a viewport, preserving the
 * current log/linear mode. Returns null for sub-4-pixel clicks/drags.
 */
export function viewFromRegion(
  view: Viewport,
  start: { x: number; y: number },
  end: { x: number; y: number },
  width: number,
  height: number,
  durationS: number,
  nyquist: number,
  minPixels: number | FrequencyBounds = 4,
  bounds?: FrequencyBounds,
): Viewport | null {
  if (!(width > 0) || !(height > 0)) {
    return null;
  }
  const activeBounds =
    bounds ?? (typeof minPixels === "number" ? undefined : minPixels);
  const minPixelsValue = typeof minPixels === "number" ? minPixels : 4;
  const x0 = Math.min(Math.max(Math.min(start.x, end.x), 0), width);
  const x1 = Math.min(Math.max(Math.max(start.x, end.x), 0), width);
  const y0 = Math.min(Math.max(Math.min(start.y, end.y), 0), height);
  const y1 = Math.min(Math.max(Math.max(start.y, end.y), 0), height);
  if (x1 - x0 < minPixelsValue || y1 - y0 < minPixelsValue) {
    return null;
  }
  const t0 = view.t0 + (x0 / width) * (view.t1 - view.t0);
  const t1 = view.t0 + (x1 / width) * (view.t1 - view.t0);
  const f0 = freqAtFraction(1 - y1 / height, view);
  const f1 = freqAtFraction(1 - y0 / height, view);
  if (!(t1 > t0) || !(f1 > f0)) {
    return null;
  }
  return clampView(
    { ...view, t0, t1, f0, f1 },
    durationS,
    nyquist,
    activeBounds,
  );
}

export function timeToX(time: number, view: Viewport, width: number): number {
  const span = view.t1 - view.t0;
  if (!(span > 0)) {
    return 0;
  }
  return ((time - view.t0) / span) * width;
}

export function xToTime(x: number, view: Viewport, width: number): number {
  if (!(width > 0)) {
    return view.t0;
  }
  return view.t0 + (x / width) * (view.t1 - view.t0);
}

export function freqToY(
  hz: number,
  view: Viewport,
  height: number,
  logMin = LOG_MIN_HZ,
): number {
  const fraction = freqFraction(hz, view, logMin);
  return (1 - fraction) * height;
}

export function yToFreq(
  y: number,
  view: Viewport,
  height: number,
  logMin = LOG_MIN_HZ,
): number {
  if (!(height > 0)) {
    return view.f0;
  }
  return freqAtFraction(1 - y / height, view, logMin);
}

export function freqFraction(
  hz: number,
  view: Viewport,
  logMin = LOG_MIN_HZ,
): number {
  const { f0, f1, logFreq } = view;
  if (!(f1 > f0)) {
    return 0.5;
  }
  if (logFreq) {
    const lo = logFloor(f0, f1, logMin);
    const top = f1 > lo ? f1 : lo * 2;
    if (!(top > lo) || !Number.isFinite(hz)) {
      return 0.5;
    }
    return (
      (Math.log(Math.max(hz, lo)) - Math.log(lo)) /
      (Math.log(top) - Math.log(lo))
    );
  }
  return (hz - f0) / (f1 - f0);
}

export function freqAtFraction(
  fraction: number,
  view: Viewport,
  logMin = LOG_MIN_HZ,
): number {
  const t = Math.min(1, Math.max(0, fraction));
  if (view.logFreq) {
    const lo = logFloor(view.f0, view.f1, logMin);
    const top = view.f1 > lo ? view.f1 : lo * 2;
    return Math.exp(
      Math.log(lo) + t * (Math.log(top) - Math.log(lo)),
    );
  }
  return view.f0 + t * (view.f1 - view.f0);
}

/** Rectangle of the current view inside a full-spectrum mini-map. */
export function minimapRect(
  view: Viewport,
  durationS: number,
  nyquist: number,
  width: number,
  height: number,
  bounds?: FrequencyBounds,
): MinimapRect {
  const full = fitView(
    durationS,
    bounds ? bounds.maxHz : nyquist,
    view.logFreq,
    bounds ? bounds.minHz : 0,
  );
  const x = timeToX(view.t0, full, width);
  const x2 = timeToX(view.t1, full, width);
  const yTop = freqToY(view.f1, full, height);
  const yBottom = freqToY(view.f0, full, height);
  return {
    x: Math.max(0, Math.min(width, x)),
    y: Math.max(0, Math.min(height, Math.min(yTop, yBottom))),
    w: Math.max(2, Math.min(width, x2) - Math.max(0, x)),
    h: Math.max(2, Math.abs(yBottom - yTop)),
  };
}

/** Centre the current view span on a mini-map click (0..1 coordinates). */
export function viewFromMinimap(
  xFraction: number,
  yFraction: number,
  view: Viewport,
  durationS: number,
  nyquist: number,
  bounds?: FrequencyBounds,
): Viewport {
  const tSpan = view.t1 - view.t0;
  const centerT = xFraction * durationS;
  const t0 = centerT - tSpan / 2;

  const full = fitView(
    durationS,
    bounds ? bounds.maxHz : nyquist,
    view.logFreq,
    bounds ? bounds.minHz : 0,
  );
  const centerFreq = freqAtFraction(1 - yFraction, full);
  let f0: number;
  let f1: number;
  if (view.logFreq) {
    // Scale both edges by the same ratio so the log zoom level is preserved.
    const currentCenter = freqAtFraction(0.5, view);
    const ratio =
      currentCenter > 0 && Number.isFinite(currentCenter)
        ? centerFreq / currentCenter
        : 1;
    f0 = view.f0 * ratio;
    f1 = view.f1 * ratio;
  } else {
    const fSpan = view.f1 - view.f0;
    f0 = centerFreq - fSpan / 2;
    f1 = f0 + fSpan;
  }
  return clampView(
    { ...view, t0, t1: t0 + tSpan, f0, f1 },
    durationS,
    nyquist,
    bounds,
  );
}

/** dB from a 0..1 fraction down the colorbar (0 = max, 1 = min). */
export function dbFromFraction(fraction: number, minDb: number, maxDb: number): number {
  const t = Math.min(1, Math.max(0, fraction));
  return maxDb - t * (maxDb - minDb);
}

export function fractionFromDb(db: number, minDb: number, maxDb: number): number {
  if (!(maxDb > minDb)) {
    return 0;
  }
  return Math.min(1, Math.max(0, (maxDb - db) / (maxDb - minDb)));
}

/** Nearest tracked order for a frequency at a given RPM. */
export function nearestOrder(
  freqHz: number,
  rpm: number | null | undefined,
  orders: readonly number[],
): { order: number; target: number } | null {
  if (!rpm || rpm <= 0 || orders.length === 0 || !Number.isFinite(freqHz)) {
    return null;
  }
  const actualOrder = (freqHz * 60) / rpm;
  let nearest = orders[0];
  let best = Math.abs(actualOrder - nearest);
  for (const order of orders.slice(1)) {
    const distance = Math.abs(actualOrder - order);
    if (distance < best) {
      best = distance;
      nearest = order;
    }
  }
  return { order: nearest, target: (nearest * rpm) / 60 };
}
