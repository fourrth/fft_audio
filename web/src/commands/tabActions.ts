import { createContext } from "react";

export interface TabActions {
  requestClose: (panelId: string) => void;
  rename: (panelId: string, name: string) => void;
  newLauncher: (panelId?: string) => void;
  toggleInspector: (displayId: number) => void;
  duplicate: (panelId: string) => void;
}

export const TabActionsContext = createContext<TabActions | null>(null);
