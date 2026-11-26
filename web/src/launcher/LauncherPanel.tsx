import { createContext, useContext } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { Launcher } from "./Launcher";
import type { LauncherActions } from "./recent";

export type { LauncherActions } from "./recent";

export const LauncherActionsContext = createContext<LauncherActions | null>(
  null,
);

/** Dockview panel adapter: the launcher UI gets callbacks via React context. */
export function LauncherPanel(props: IDockviewPanelProps) {
  const actions = useContext(LauncherActionsContext);
  if (!actions) {
    return <div className="panel-empty">Launcher is not ready.</div>;
  }
  return (
    <Launcher
      recentFiles={actions.recentFiles}
      scriptingEnabled={actions.scriptingEnabled}
      onOpenSpectrogram={() => actions.openSpectrogram(props.api.id)}
      onOpenInSplit={() => actions.openSpectrogramInSplit(props.api.id)}
      onOpenRecorder={() => actions.openRecorder(props.api.id)}
      onOpenScripting={() => actions.openScripting(props.api.id)}
      onOpenFile={(path) => actions.openFile(props.api.id, path)}
    />
  );
}
