import { useContext } from "react";
import type { IDockviewHeaderActionsProps } from "dockview-react";
import { TabActionsContext } from "../commands/tabActions";

/** Group-header `+`: always adds a Launcher tab to the right of the active tab. */
export function AddTabButton({
  activePanel,
}: IDockviewHeaderActionsProps) {
  const actions = useContext(TabActionsContext);
  return (
    <button
      type="button"
      className="add-tab"
      aria-label="Add tab"
      data-ui-id="shell.add-tab"
      title="New Launcher"
      disabled={!actions}
      onClick={() => actions?.newLauncher(activePanel?.id)}
    >
      +
    </button>
  );
}
