import { useEffect, useRef, useState } from "react";
import type { Transport } from "../transport/types";
import type { CutsPayload } from "./model";

export interface ChildCutsState {
  payload: CutsPayload | null;
  phase: "loading" | "ready" | "error";
  error: string | null;
  stale: boolean;
  reload: () => void;
}

/**
 * Compute cuts for a child display and refresh whenever the parent finishes
 * a new analysis. `depsKey` should summarize settings/lines (JSON works).
 */
export function useChildCuts(
  parentId: number | null,
  transport: Transport | null,
  compute: () => Promise<CutsPayload>,
  depsKey: string,
): ChildCutsState {
  const [payload, setPayload] = useState<CutsPayload | null>(null);
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState<string | null>(null);
  const [stale, setStale] = useState(false);
  const [reloadToken, setReloadToken] = useState(0);
  const computeRef = useRef(compute);
  computeRef.current = compute;

  useEffect(() => {
    if (parentId === null || !transport) {
      setPhase("error");
      setError("This child has no parent Spectrogram.");
      return;
    }
    let disposed = false;
    setPhase((current) => (current === "ready" ? current : "loading"));
    setError(null);

    const load = async () => {
      try {
        const result = await computeRef.current();
        if (!disposed) {
          setPayload(result);
          setPhase("ready");
          setStale(false);
        }
      } catch (reason) {
        if (!disposed) {
          setError(reason instanceof Error ? reason.message : String(reason));
          setPhase("error");
          setStale(false);
        }
      }
    };

    void load();
    const offDone = transport.on("analysis.done", (event) => {
      if (event.displayId === parentId) {
        setStale(true);
        void load();
      }
    });
    const offProgress = transport.on("analysis.progress", (event) => {
      if (event.displayId === parentId) {
        setStale(true);
      }
    });
    const offError = transport.on("analysis.error", (event) => {
      if (event.displayId === parentId) {
        setError(event.message);
        setPhase("error");
        setStale(false);
      }
    });

    return () => {
      disposed = true;
      offDone();
      offProgress();
      offError();
    };
    // depsKey summarizes the settings/lines that change the request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [depsKey, parentId, reloadToken, transport]);

  return {
    payload,
    phase,
    error,
    stale,
    reload: () => setReloadToken((value) => value + 1),
  };
}
