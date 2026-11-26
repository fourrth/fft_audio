/** Messages exchanged between the browser Transport and its WASM worker. */

export interface WorkerInit {
  kind: "init";
  wasmBase: string;
}

export interface WorkerRequest {
  kind: "request";
  id: number;
  command: string;
  payload?: unknown;
}

export type WorkerInbound = WorkerInit | WorkerRequest;

export interface WorkerSuccess {
  kind: "response";
  id: number;
  ok: true;
  result: unknown;
}

export interface WorkerFailure {
  kind: "response";
  id: number;
  ok: false;
  error: string;
}

export interface WorkerEvent {
  kind: "event";
  event: string;
  payload: unknown;
}

export type WorkerOutbound = WorkerSuccess | WorkerFailure | WorkerEvent;

/** The subset of the generated wasm-bindgen module used by the worker. */
export interface WasmEngine {
  open(
    displayId: number,
    name: string,
    bytes: Uint8Array,
    settingsJson: string,
  ): void;
  open_with_progress(
    displayId: number,
    name: string,
    bytes: Uint8Array,
    settingsJson: string,
    progress: (framesDone: number, framesTotal: number | null) => void,
  ): void;
  payload(displayId: number, maxFrames: number, maxBins: number): string;
  export_png(displayId: number, optionsJson: string): Uint8Array;
  order_cuts(requestJson: string): string;
  line_cuts(requestJson: string): string;
  cuts_csv(cutsJson: string): string;
  theme_get(id: string, customJson: string): string;
  theme_list(customJson: string): string;
  theme_duplicate(
    source: string,
    name: string,
    customJson: string,
  ): string;
  close(displayId: number): void;
}
