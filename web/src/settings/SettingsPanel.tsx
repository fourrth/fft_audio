import { useContext } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { DisplayActionsContext } from "../display/DisplayPanel";
import { SettingsContent } from "./SettingsContent";

/** Dockview panel adapter for the six-page Settings view. */
export function SettingsPanel(props: IDockviewPanelProps) {
  const actions = useContext(DisplayActionsContext);
  return (
    <SettingsContent
      transport={actions?.transport ?? null}
      onClose={() => props.api.close()}
    />
  );
}
