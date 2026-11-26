import type { LineSpec } from "../cuts/model";
import type { OrderCutSettings } from "../cuts/settings";

export interface SpectrogramLineCut {
  panelId: string;
  lineIndex: number;
  line: LineSpec;
  color: string;
  snap: boolean;
  snapDeg: number;
}

export interface SpectrogramOrderSource {
  panelId: string;
  settings: OrderCutSettings;
}

export interface SpectrogramOrderGuidePoint {
  timeS: number;
  freqHz: number;
}

export interface SpectrogramOrderGuide {
  id: string;
  panelId: string;
  order: number;
  color: string;
  points: SpectrogramOrderGuidePoint[];
}
