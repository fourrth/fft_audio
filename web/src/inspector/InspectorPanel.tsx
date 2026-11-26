import { useContext, useEffect, useMemo, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { DisplayActionsContext } from "../display/DisplayPanel";
import { LauncherActionsContext } from "../launcher/LauncherPanel";
import type { ShellDisplay } from "../shell/session";
import { Banner } from "../ui/Banner";
import { useToast } from "../ui/toast";
import { InspectorContent } from "./InspectorContent";

export interface InspectorParams {
  displayId: number;
}

/** Dockview panel adapter: finds its target display and syncs its params. */
export function InspectorPanel(props: IDockviewPanelProps<InspectorParams>) {
  const displayId = props.params?.displayId;
  const actions = useContext(DisplayActionsContext);
  const launcherActions = useContext(LauncherActionsContext);
  const { pushToast } = useToast();
  const [version, setVersion] = useState(0);

  const target = useMemo(() => {
    if (displayId === undefined) {
      return undefined;
    }
    return props.containerApi.panels.find((panel) => {
      const display = (panel.params as { display?: ShellDisplay } | undefined)
        ?.display;
      return display?.id === displayId;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [displayId, props.containerApi, version]);

  const display = (target?.params as { display?: ShellDisplay } | undefined)
    ?.display;

  useEffect(() => {
    if (!target) {
      return;
    }
    const subscription = target.api.onDidParametersChange(() => {
      setVersion((value) => value + 1);
    });
    return () => subscription.dispose();
  }, [target]);

  if (!display) {
    return (
      <div className="inspector">
        <div className="inspector-header">
          <strong>Inspector</strong>
        </div>
        <div style={{ padding: 10 }}>
          <Banner tone="warn">
            The target display was closed. Close this inspector tab.
          </Banner>
        </div>
      </div>
    );
  }

  const updateState = (patch: Record<string, unknown>) => {
    target?.api.updateParameters({
      display: {
        ...display,
        state: { ...display.state, ...patch },
      },
    });
    actions?.saveSession();
    setVersion((value) => value + 1);
  };

  const browse = async () => {
    if (!actions || !target) {
      return;
    }
    try {
      const path = await actions.browseForAudio();
      if (!path) {
        return;
      }
      if (display?.kind === "launcher" && launcherActions) {
        launcherActions.openFile(target.id, path);
      } else {
        actions.openPath(target.id, path);
      }
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not open the picker: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  return (
    <InspectorContent
      display={display}
      transport={actions?.transport ?? null}
      onUpdateState={updateState}
      onBrowse={() => void browse()}
      onClose={() => props.api.close()}
    />
  );
}
