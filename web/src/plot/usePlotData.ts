import { useEffect, useState } from "react";
import type { Transport } from "../transport/types";
import type { PlotLayout, TablePlot } from "./types";

export interface PlotDataState {
  payload: TablePlot | null;
  phase: "loading" | "ready" | "error";
  error: string | null;
  reload: () => void;
}

/**
 * Load x/y series for a Plot display. The request key is the table path plus
 * the column layout, so switching the layout re-reads the same file.
 */
export function usePlotData(
  transport: Transport | null,
  path: string,
  layout: PlotLayout,
): PlotDataState {
  const [payload, setPayload] = useState<TablePlot | null>(null);
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  useEffect(() => {
    if (!transport) {
      setPayload(null);
      setPhase("error");
      setError("This build cannot read plot files.");
      return;
    }
    if (!path) {
      setPayload(null);
      setPhase("error");
      setError("This Plot display has no file.");
      return;
    }
    let disposed = false;
    setPhase("loading");
    setError(null);

    transport
      .invoke({ name: "plot.load", args: { path, layout } })
      .then((result) => {
        if (disposed) {
          return;
        }
        setPayload(result);
        setPhase("ready");
      })
      .catch((reason: unknown) => {
        if (disposed) {
          return;
        }
        setPayload(null);
        setError(reason instanceof Error ? reason.message : String(reason));
        setPhase("error");
      });

    return () => {
      disposed = true;
    };
  }, [layout, path, reloadToken, transport]);

  return {
    payload,
    phase,
    error,
    reload: () => setReloadToken((value) => value + 1),
  };
}
