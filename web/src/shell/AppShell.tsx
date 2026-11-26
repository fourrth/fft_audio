import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  DockviewReact,
  type DockviewApi,
  type DockviewReadyEvent,
  type DockviewTheme,
  type SerializedDockview,
} from "dockview-react";
import "dockview-react/dist/styles/dockview.css";
import type { AppInfo, DisplayInfo, Transport } from "../transport/types";
import {
  LauncherActionsContext,
  LauncherPanel,
  type LauncherActions,
} from "../launcher/LauncherPanel";
import { addRecentFile, fileName } from "../launcher/recent";
import type { Cut, LineSpec } from "../cuts/model";
import { readLines } from "../cuts/settings";
import type { PlotLayout } from "../plot/types";
import {
  DisplayActionsContext,
  DisplayPanel,
  type DisplayActions,
  type LineCutSibling,
} from "../display/DisplayPanel";
import { AddTabButton } from "../display/AddTabButton";
import {
  canSpawnChild,
  makeChildDisplay,
  type ChildKind,
} from "../display/children";
import { InspectorPanel } from "../inspector/InspectorPanel";
import { SettingsPanel } from "../settings/SettingsPanel";
import { CommandPalette } from "../commands/CommandPalette";
import { ShortcutsModal } from "../commands/ShortcutsModal";
import { FftTab } from "../commands/FftTab";
import {
  TabActionsContext,
  type TabActions,
} from "../commands/tabActions";
import {
  CommandRegistry,
  shortcutMatches,
  type UiCommand,
} from "../commands/registry";
import {
  UNDO_TTL_MS,
  planClose,
  savePreset,
  undoAvailable,
  uniquePresetName,
  type UndoSnapshot,
} from "../commands/layout";
import { loadAndApplyTheme } from "../theme";
import { applyAppearance } from "../settings/model";
import { NotificationDrawer } from "../ui/NotificationDrawer";
import { ToastStack } from "../ui/ToastStack";
import { useToast } from "../ui/toast";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import {
  createSession,
  displayFromPanel,
  parseSession,
  type SessionDocument,
  type ShellDisplay,
} from "./session";
import {
  SCRIPTED_DISPLAY_ID_BASE,
  closedDisplayTreeIds,
  closedTreePanelIds,
  displayAdoptionOrder,
  isBackendOwnedDisplay,
  isEphemeralDisplay,
  nextUserDisplayId,
  panelIdForDisplay,
  recreatedUserDisplay,
  restoreDisposition,
  restoreDropIds,
  sanitizeNextId,
  upsertDisplay,
} from "./adoption";
import "./shell.css";

const DOCK_COMPONENTS = {
  launcher: LauncherPanel,
  display: DisplayPanel,
  inspector: InspectorPanel,
  settings: SettingsPanel,
};

/**
 * Drop restored Scripting displays (and their Console children) when this
 * build has no scripting UI. Inspector panels use `inspector-${displayId}`,
 * so they follow their display.
 */
function dropScriptingPanels(api: DockviewApi): void {
  const displays = api.panels.flatMap((panel) => {
    const display = (panel.params as { display?: ShellDisplay } | undefined)
      ?.display;
    return display ? [display] : [];
  });
  const removed = new Set<number>();
  for (const display of displays) {
    if (display.kind === "scripting" || display.kind === "script_console") {
      for (const id of closedDisplayTreeIds(display.id, [], displays)) {
        removed.add(id);
      }
    }
  }
  for (const panel of [...api.panels]) {
    const display = (panel.params as { display?: ShellDisplay } | undefined)
      ?.display;
    if (display && removed.has(display.id)) {
      api.removePanel(panel);
    }
  }
  for (const id of removed) {
    const inspector = api.getPanel(`inspector-${id}`);
    if (inspector) {
      api.removePanel(inspector);
    }
  }
}

export interface AppShellProps {
  /** Null in a plain browser preview: the shell renders without persistence. */
  transport: Transport | null;
}

type DisplayAdoptionEvent =
  | { type: "open" | "updated"; display: DisplayInfo }
  | { type: "closed"; id: number; children: number[] };

export function AppShell({ transport }: AppShellProps) {
  const apiRef = useRef<DockviewApi | null>(null);
  const layoutDisposable = useRef<{ dispose(): void } | null>(null);
  const activeDisposable = useRef<{ dispose(): void } | null>(null);
  const saveTimer = useRef<number | undefined>(undefined);
  const restoring = useRef(false);
  const nextId = useRef(1);
  const recentRef = useRef<string[]>([]);
  const presetsRef = useRef<Record<string, unknown>>({});
  const pendingClose = useRef<{
    layout: unknown;
    panelIds: string[];
    displayIds: number[];
    childCount: number;
    undoable: boolean;
    backendRoot: boolean;
  } | null>(null);
  const undoRef = useRef<UndoSnapshot | null>(null);
  const pendingDisplayEvents = useRef<DisplayAdoptionEvent[]>([]);
  const localCloseIds = useRef(new Set<number>());
  const deferredCloses = useRef(new Map<number, number>());
  /** Layout events caused only by an ephemeral panel must not schedule a save. */
  const suppressLayoutSave = useRef(false);

  const [status, setStatus] = useState("Ready");
  const [activePanelKind, setActivePanelKind] = useState<
    ShellDisplay["kind"] | null
  >(null);
  const [recentFiles, setRecentFiles] = useState<string[]>([]);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [theme, setTheme] = useState<{
    name: string;
    appearance: "dark" | "light";
  }>({ name: "Dark", appearance: "dark" });
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [plotPalette, setPlotPalette] = useState<string[]>([]);
  const [layoutPresets, setLayoutPresets] = useState<Record<string, unknown>>({});
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [presetDialogOpen, setPresetDialogOpen] = useState(false);
  const [presetName, setPresetName] = useState("");
  const [confirmClose, setConfirmClose] = useState<{
    panelId: string;
    displayId: number;
    childCount: number;
  } | null>(null);
  const { pushToast, unreadCount } = useToast();

  const mode = appInfo?.mode ?? transport?.mode ?? "preview";
  const scriptingEnabled = appInfo?.capabilities.scripting != null;

  useEffect(() => {
    if (!transport) {
      return;
    }
    let cancelled = false;
    void transport
      .invoke({ name: "app.info", args: {} })
      .then((info) => {
        if (!cancelled) {
          setAppInfo(info);
        }
      })
      .catch((error: unknown) => {
        console.error("could not read app info", error);
      });
    void (async () => {
      let themeId: string | undefined;
      try {
        const settings = await transport.invoke({ name: "settings.get", args: {} });
        if (!cancelled) {
          applyAppearance(settings.appearance);
          themeId = settings.appearance.theme;
        }
      } catch (error) {
        console.error("could not read settings", error);
      }
      try {
        const resolved = await loadAndApplyTheme(transport, themeId);
        if (!cancelled) {
          setTheme({ name: resolved.name, appearance: resolved.appearance });
          setPlotPalette(resolved.plotPalette);
          setStatus(`Theme: ${resolved.name}`);
        }
      } catch (error) {
        console.error("could not apply the saved theme", error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [transport]);

  const addLauncher = useCallback((api: DockviewApi) => {
    nextId.current = Math.max(nextId.current, 2);
    const display: ShellDisplay = {
      id: 1,
      parent: null,
      kind: "launcher",
      name: "Launcher",
      state: {},
    };
    api.addPanel({
      id: "launcher-1",
      component: "launcher",
      title: "Launcher",
      params: { display },
    });
  }, []);

  /** Allocate below the reserved backend/script id range. */
  const allocateUserId = useCallback((): number | null => {
    const api = apiRef.current;
    if (nextId.current >= SCRIPTED_DISPLAY_ID_BASE) {
      console.error(
        `display id counter ${nextId.current} reached the reserved range; reusing a free user id`,
      );
    }
    const used =
      api?.panels.flatMap((panel) => {
        const display = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        return display ? [display.id] : [];
      }) ?? [];
    const id = nextUserDisplayId(nextId.current, used);
    if (id === null) {
      console.error("no free user display ids remain below the reserved range");
      pushToast({
        tone: "error",
        message: "No free display ids remain below the reserved range.",
      });
      return null;
    }
    nextId.current = id + 1;
    return id;
  }, [pushToast]);

  /**
   * Never leave an empty dock tree. Deferred to a microtask so Dockview can
   * finish its own close/layout event before we add a panel back.
   */
  const ensureLauncher = useCallback(
    (api: DockviewApi) => {
      if (restoring.current || api.panels.length !== 0) {
        return;
      }
      queueMicrotask(() => {
        if (restoring.current || apiRef.current !== api) {
          return;
        }
        if (api.panels.length === 0) {
          addLauncher(api);
        }
      });
    },
    [addLauncher],
  );

  const rememberFile = useCallback(
    (path: string) => {
      // Browser files live in worker memory only; a path in the recent list
      // could not be reopened after a reload without picking the file again.
      if (transport?.mode === "browser") {
        return;
      }
      const next = addRecentFile(recentRef.current, path);
      recentRef.current = next;
      setRecentFiles(next);
    },
    [transport],
  );

  const scheduleSave = useCallback(
    (api: DockviewApi) => {
      if (!transport || restoring.current || suppressLayoutSave.current) {
        return;
      }
      if (saveTimer.current !== undefined) {
        window.clearTimeout(saveTimer.current);
      }
      saveTimer.current = window.setTimeout(() => {
        const displays = api.panels
          .flatMap((panel) => {
            const display = (
              panel.params as { display?: ShellDisplay } | undefined
            )?.display;
            return display ? [display] : [];
          })
          .filter((display) => !isEphemeralDisplay(display));
        nextId.current = sanitizeNextId(
          nextId.current,
          displays.map((display) => display.id),
        );
        const session: SessionDocument = createSession(
          api.toJSON(),
          displays,
          nextId.current,
          recentRef.current,
          presetsRef.current,
        );
        void transport
          .invoke({ name: "session.save", args: { session } })
          .catch((error: unknown) => {
            console.error("could not save the session", error);
            setStatus("Could not save the session");
            pushToast({
              tone: "error",
              message: "Could not save the session. Changes stay in this window.",
            });
          });
      }, 250);
    },
    [pushToast, transport],
  );

  const addLauncherTab = useCallback(
    (panelId?: string) => {
      const api = apiRef.current;
      if (!api) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      let sequence = 2;
      while (api.getPanel(`launcher-${sequence}`)) {
        sequence += 1;
      }
      const reference = panelId ? api.getPanel(panelId) : api.activePanel;
      const index = reference
        ? reference.group.panels.indexOf(reference) + 1
        : undefined;
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "launcher",
        name: "Launcher",
        state: {},
      };
      const panel = api.addPanel({
        id: `launcher-${sequence}`,
        component: "launcher",
        title: "Launcher",
        params: { display },
        ...(reference
          ? {
              position: {
                referencePanel: reference,
                direction: "within" as const,
                index,
              },
            }
          : {}),
      });
      panel.api.setActive();
      scheduleSave(api);
      setStatus("Opened Launcher");
    },
    [allocateUserId, scheduleSave],
  );

  /**
   * Close a registered display once. The backend broadcasts
   * `display.closed`; that echo is ignored here because the panel close (and
   * possible Undo) already happened locally.
   */  const closeDisplayOnBackend = useCallback(
    (displayId: number) => {
      if (!transport) {
        return;
      }
      const deferred = deferredCloses.current.get(displayId);
      if (deferred !== undefined) {
        window.clearTimeout(deferred);
        deferredCloses.current.delete(displayId);
      }
      localCloseIds.current.add(displayId);
      window.setTimeout(() => {
        localCloseIds.current.delete(displayId);
      }, 5000);
      void transport
        .invoke({ name: "display.close", args: { displayId } })
        .catch((error: unknown) => {
          console.error(`could not close display ${displayId}`, error);
        });
    },
    [transport],
  );

  const removeClosedPanels = useCallback(
    (api: DockviewApi, id: number, children: readonly number[]) => {
      const displays = api.panels.flatMap((panel) => {
        const display = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        return display ? [display] : [];
      });
      const removedIds = closedTreePanelIds(id, children, displays);
      const persistent = removedIds.some((panelId) => {
        const display = (
          api.getPanel(panelId)?.params as
            | { display?: ShellDisplay }
            | undefined
        )?.display;
        return display ? !isEphemeralDisplay(display) : false;
      });
      if (!persistent) {
        suppressLayoutSave.current = true;
      }
      let removed = false;
      for (const panelId of removedIds) {
        const panel = api.getPanel(panelId);
        if (panel) {
          api.removePanel(panel);
          removed = true;
        }
        const displayId = Number(panelId.slice("panel-".length));
        const inspector = api.getPanel(`inspector-${displayId}`);
        if (inspector) {
          api.removePanel(inspector);
          removed = true;
        }
      }
      if (!persistent) {
        queueMicrotask(() => {
          suppressLayoutSave.current = false;
        });
      }
      ensureLauncher(api);
      if (removed && persistent) {
        scheduleSave(api);
      }
    },
    [ensureLauncher, scheduleSave],
  );

  const applyDisplayEvent = useCallback(
    (event: DisplayAdoptionEvent) => {
      const api = apiRef.current;
      if (!api || restoring.current) {
        pendingDisplayEvents.current.push(event);
        return;
      }

      if (event.type === "closed") {
        const wasLocal = localCloseIds.current.delete(event.id);
        if (wasLocal) {
          // The panel was closed by this window; an Undo may already have
          // restored it, so a late echo must not delete it again.
          return;
        }
        // Backend ids are never reused, so their close is final.
        if (event.id >= SCRIPTED_DISPLAY_ID_BASE) {
          removeClosedPanels(api, event.id, event.children);
          return;
        }
        // A user display can be replaced by a new `spectrogram.open` with the
        // same id. The backend closes the old record first, then opens the new
        // one; keep the panel until we know no matching open follows.
        const existingTimer = deferredCloses.current.get(event.id);
        if (existingTimer !== undefined) {
          window.clearTimeout(existingTimer);
        }
        const timer = window.setTimeout(() => {
          deferredCloses.current.delete(event.id);
          const current = apiRef.current;
          if (current) {
            removeClosedPanels(current, event.id, event.children);
          }
        }, 500);
        deferredCloses.current.set(event.id, timer);
        return;
      }

      const deferred = deferredCloses.current.get(event.display.id);
      if (deferred !== undefined) {
        window.clearTimeout(deferred);
        deferredCloses.current.delete(event.display.id);
      }
      if (localCloseIds.current.has(event.display.id)) {
        // The user just closed this panel; a late open/update from the backend
        // must not resurrect it.
        return;
      }

      const panelId = panelIdForDisplay(event.display.id);
      const existingPanel = api.getPanel(panelId);
      const existing = (
        existingPanel?.params as { display?: ShellDisplay } | undefined
      )?.display;
      const upsert = upsertDisplay(existing, event.display);

      const ephemeral = isEphemeralDisplay(upsert.display);
      if (ephemeral && !existingPanel) {
        suppressLayoutSave.current = true;
      }
      if (existingPanel) {
        existingPanel.api.updateParameters({ display: upsert.display });
        existingPanel.api.setTitle(upsert.display.name);
      } else {
        const parentId = event.display.parent;
        const parentPanel =
          parentId === null
            ? undefined
            : api.panels.find((panel) => {
                const candidate = (
                  panel.params as { display?: ShellDisplay } | undefined
                )?.display;
                return candidate?.id === parentId;
              });
        api.addPanel({
          id: panelId,
          component: "display",
          title: upsert.display.name,
          params: { display: upsert.display },
          ...(parentPanel
            ? { position: { referencePanel: parentPanel, direction: "below" as const } }
            : {}),
        });
      }
      if (ephemeral && !existingPanel) {
        queueMicrotask(() => {
          suppressLayoutSave.current = false;
        });
      }

      if (!ephemeral) {
        scheduleSave(api);
      }
    },
    [removeClosedPanels, scheduleSave],
  );

  const flushDisplayEvents = useCallback(() => {
    const pending = pendingDisplayEvents.current;
    pendingDisplayEvents.current = [];
    for (const event of pending) {
      applyDisplayEvent(event);
    }
  }, [applyDisplayEvent]);

  /**
   * Reconcile the backend registry after the session has been restored.
   *
   * A script or external client can create displays before this window
   * connects; those `display.open` events are gone, but the registry still has
   * the records. Keep restore gating active while the snapshot is fetched,
   * then adopt it and replay any events that arrived in the meantime.
   */
  const syncExistingDisplays = useCallback(async () => {
    let displays: DisplayInfo[] = [];
    if (transport) {
      try {
        displays = await transport.invoke({ name: "display.list", args: {} });
      } catch (error) {
        console.error("could not read the display registry", error);
      }
    }

    restoring.current = false;
    try {
      for (const display of displayAdoptionOrder(displays)) {
        applyDisplayEvent({ type: "open", display });
      }
    } finally {
      flushDisplayEvents();
    }
  }, [applyDisplayEvent, flushDisplayEvents, transport]);

  const onReady = useCallback(
    (event: DockviewReadyEvent) => {
      const api = event.api;
      apiRef.current = api;
      const syncActiveKind = () => {
        const active = api.activePanel;
        const display = (
          active?.params as { display?: ShellDisplay } | undefined
        )?.display;
        setActivePanelKind(display?.kind ?? null);
      };
      activeDisposable.current?.dispose();
      activeDisposable.current = api.onDidActivePanelChange(syncActiveKind);
      layoutDisposable.current?.dispose();
      layoutDisposable.current = api.onDidLayoutChange(() => {
        scheduleSave(api);
        ensureLauncher(api);
        syncActiveKind();
        window.dispatchEvent(new Event("fft:layout-change"));
      });
      syncActiveKind();

      if (!transport) {
        addLauncher(api);
        return;
      }

      restoring.current = true;
      void Promise.all([
        transport.invoke({ name: "settings.get", args: {} }),
        transport.invoke({ name: "session.get", args: {} }),
        transport.invoke({ name: "app.info", args: {} }),
      ])
        .then(([settings, savedValue, info]) => {
          setAppInfo(info);
          const saved = settings.session.reopen_last
            ? parseSession(savedValue)
            : null;
          if (saved) {
            recentRef.current = saved.recentFiles;
            setRecentFiles(saved.recentFiles);
            presetsRef.current = saved.layoutPresets;
            setLayoutPresets(saved.layoutPresets);
          }
          if (saved?.layout) {
            try {
              api.fromJSON(saved.layout as SerializedDockview);
            } catch (error) {
              console.error("could not restore the dock layout", error);
              setStatus("Saved layout was invalid; starting fresh");
            }
          }
          if (info.capabilities.scripting == null) {
            dropScriptingPanels(api);
          }

          // Backend/script display records cannot be attached after a restart.
          // Recreate file-backed spectrograms as user displays; drop the rest,
          // including local child cuts under a dropped backend parent.
          const restored = api.panels.flatMap((panel) => {
            const display = (
              panel.params as { display?: ShellDisplay } | undefined
            )?.display;
            return display ? [{ panelId: panel.id, display }] : [];
          });
          const dropped = restoreDropIds(restored.map(({ display }) => display));
          const recreated: ShellDisplay[] = [];
          const recreatedIds = new Map<number, number>();
          const kept: typeof restored = [];
          const removeRestoredPanel = (panelId: string, displayId: number) => {
            const panel = api.getPanel(panelId);
            if (panel) {
              api.removePanel(panel);
            }
            const inspector = api.getPanel(`inspector-${displayId}`);
            if (inspector) {
              api.removePanel(inspector);
            }
          };
          for (const { panelId, display } of restored) {
            if (dropped.has(display.id)) {
              removeRestoredPanel(panelId, display.id);
              continue;
            }
            const disposition = restoreDisposition(display);
            if (disposition === "recreate") {
              const id = allocateUserId();
              if (id !== null) {
                recreatedIds.set(display.id, id);
                recreated.push(recreatedUserDisplay(display, id));
              }
              removeRestoredPanel(panelId, display.id);
            } else {
              kept.push({ panelId, display });
            }
          }
          // Children of a recreated parent follow its new user id; keeps a
          // local child display functional when its file-backed parent is
          // re-opened under a fresh id.
          for (const { panelId, display } of kept) {
            const newParent =
              display.parent === null
                ? undefined
                : recreatedIds.get(display.parent);
            if (newParent === undefined) {
              continue;
            }
            const panel = api.getPanel(panelId);
            if (panel) {
              panel.api.updateParameters({
                display: { ...display, parent: newParent },
              });
            }
          }
          for (const display of recreated) {
            api.addPanel({
              id: panelIdForDisplay(display.id),
              component: "display",
              title: display.name,
              params: { display },
            });
          }

          if (api.panels.length === 0) {
            addLauncher(api);
          }
          const userIds = api.panels.flatMap((panel) => {
            const display = (
              panel.params as { display?: ShellDisplay } | undefined
            )?.display;
            return display ? [display.id] : [];
          });
          if (saved && saved.nextId >= SCRIPTED_DISPLAY_ID_BASE) {
            console.error(
              `saved session nextId ${saved.nextId} is in the reserved display range; clamping`,
            );
          }
          nextId.current = sanitizeNextId(
            Math.max(nextId.current, saved?.nextId ?? 1),
            userIds,
          );
          void syncExistingDisplays().then(() => {
            setStatus(saved ? "Session restored" : "Ready");
            scheduleSave(api);
          });
        })
        .catch((error: unknown) => {
          console.error("could not restore the session", error);
          if (api.panels.length === 0) {
            addLauncher(api);
          }
          void syncExistingDisplays().then(() => {
            setStatus("Could not restore the session; starting fresh");
            scheduleSave(api);
          });
        });
    },
    [
      addLauncher,
      allocateUserId,
      ensureLauncher,
      scheduleSave,
      syncExistingDisplays,
      transport,
    ],
  );

  useEffect(() => {
    if (!transport) {
      return;
    }
    const offOpen = transport.on("display.open", ({ display }) => {
      applyDisplayEvent({ type: "open", display });
    });
    const offUpdated = transport.on("display.updated", ({ display }) => {
      applyDisplayEvent({ type: "updated", display });
    });
    const offClosed = transport.on("display.closed", ({ id, children }) => {
      applyDisplayEvent({ type: "closed", id, children });
    });
    return () => {
      offOpen();
      offUpdated();
      offClosed();
    };
  }, [applyDisplayEvent, transport]);

  const addSpectrogram = useCallback(
    (path?: string) => {
      const api = apiRef.current;
      if (!api) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const name = path ? fileName(path) : `Spectrogram ${id}`;
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "spectrogram",
        name,
        state: path
          ? {
              path,
              ...(transport?.mode === "browser" ? { ephemeral: true } : {}),
            }
          : {},
      };
      api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: display.name,
        params: { display },
      });
      if (path) {
        rememberFile(path);
      }
      setStatus(`Opened ${display.name}`);
      pushToast({ tone: "info", message: `Opened ${display.name}` });
    },
    [allocateUserId, pushToast, rememberFile, transport],
  );

  const addPlotDisplay = useCallback(
    (path: string, layout: PlotLayout) => {
      const api = apiRef.current;
      if (!api) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "plot",
        name: fileName(path),
        state: {
          path,
          plotLayout: layout,
          ...(transport?.mode === "browser" ? { ephemeral: true } : {}),
        },
      };
      api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: display.name,
        params: { display },
      });
      setStatus(`Opened ${display.name}`);
      pushToast({ tone: "info", message: `Opened ${display.name}` });
    },
    [allocateUserId, pushToast, transport],
  );

  const replaceLauncher = useCallback(
    (panelId: string, path?: string) => {
      const api = apiRef.current;
      if (!api) {
        return;
      }
      const launcher = api.getPanel(panelId);
      if (!launcher) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const name = path ? fileName(path) : `Spectrogram ${id}`;
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "spectrogram",
        name,
        state: path
          ? {
              path,
              ...(transport?.mode === "browser" ? { ephemeral: true } : {}),
            }
          : {},
      };
      const index = launcher.group.panels.indexOf(launcher);
      const panel = api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: name,
        params: { display },
        position: {
          referencePanel: launcher,
          direction: "within",
          index: index >= 0 ? index : undefined,
        },
      });
      api.removePanel(launcher);
      panel.api.setActive();
      if (path) {
        rememberFile(path);
        setStatus(`Opened ${fileName(path)}`);
        pushToast({ tone: "info", message: `Opened ${fileName(path)}` });
      } else {
        setStatus(`Opened ${name}`);
        pushToast({ tone: "info", message: `Opened ${name}` });
      }
    },
    [allocateUserId, pushToast, rememberFile, transport],
  );

  const spawnSpectrogram = useCallback(() => {
    const active = apiRef.current?.activePanel;
    const display = (
      active?.params as { display?: ShellDisplay } | undefined
    )?.display;
    if (active && display?.kind === "launcher") {
      replaceLauncher(active.id);
      return;
    }
    addSpectrogram();
  }, [addSpectrogram, replaceLauncher]);

  const openSpectrogramInSplit = useCallback(
    (panelId: string) => {
      const api = apiRef.current;
      const launcher = api?.getPanel(panelId);
      if (!api || !launcher) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "spectrogram",
        name: `Spectrogram ${id}`,
        state: {},
      };
      api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: display.name,
        params: { display },
        position: { referencePanel: launcher, direction: "right" },
      });
      setStatus(`Opened ${display.name} in a split`);
      pushToast({ tone: "info", message: `Opened ${display.name} in a split` });
    },
    [allocateUserId, pushToast],
  );

  const openScripting = useCallback((panelId: string) => {
    const api = apiRef.current;
    const launcher = api?.getPanel(panelId);
    if (!api || !launcher) {
      return;
    }
    const id = allocateUserId();
    if (id === null) {
      return;
    }
    const display: ShellDisplay = {
      id,
      parent: null,
      kind: "scripting",
      name: "Scripting",
      state: {},
    };
    const index = launcher.group.panels.indexOf(launcher);
    const panel = api.addPanel({
      id: `panel-${id}`,
      component: "display",
      title: display.name,
      params: { display },
      position: {
        referencePanel: launcher,
        direction: "within",
        index: index >= 0 ? index : undefined,
      },
    });
    api.removePanel(launcher);
    panel.api.setActive();
    setStatus("Opened Scripting");
    pushToast({ tone: "info", message: "Opened Scripting" });
  }, [allocateUserId, pushToast]);

  const openRecorder = useCallback(
    (panelId: string) => {
      const api = apiRef.current;
      const launcher = api?.getPanel(panelId);
      if (!api || !launcher) {
        return;
      }
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const display: ShellDisplay = {
        id,
        parent: null,
        kind: "recorder",
        name: "Recorder",
        state: {},
      };
      const index = launcher.group.panels.indexOf(launcher);
      const panel = api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: display.name,
        params: { display },
        position: {
          referencePanel: launcher,
          direction: "within",
          index: index >= 0 ? index : undefined,
        },
      });
      api.removePanel(launcher);
      panel.api.setActive();
      setStatus("Opened Recorder");
      pushToast({ tone: "info", message: "Opened Recorder" });
    },
    [allocateUserId, pushToast],
  );

  const launcherActions = useMemo<LauncherActions>(
    () => ({
      recentFiles,
      scriptingEnabled,
      openSpectrogram: (panelId) => replaceLauncher(panelId),
      openSpectrogramInSplit,
      openRecorder,
      openScripting,
      openFile: (panelId, path) => replaceLauncher(panelId, path),
    }),
    [
      openRecorder,
      openScripting,
      openSpectrogramInSplit,
      recentFiles,
      replaceLauncher,
      scriptingEnabled,
    ],
  );

  const browseForAudio = useCallback(async (): Promise<string | null> => {
    if (!transport) {
      return null;
    }
    const result = await transport.invoke({ name: "dialog.openAudio", args: {} });
    return result?.path ?? null;
  }, [transport]);

  const openPath = useCallback(
    (panelId: string, path: string) => {
      const api = apiRef.current;
      const panel = api?.getPanel(panelId);
      if (!panel) {
        return;
      }
      const previous = displayFromPanel(panel, nextId.current);
      const display: ShellDisplay = {
        ...previous,
        kind: "spectrogram",
        name: fileName(path),
        state: {
          ...previous.state,
          path,
          // The next analysis replaces the previous input metadata. Keeping it
          // would briefly show table-only settings for a newly chosen audio
          // file (or the reverse) while the new payload loads.
          analysisMeta: {},
          ...(transport?.mode === "browser" ? { ephemeral: true } : {}),
        },
      };
      panel.api.updateParameters({ display });
      panel.api.setTitle(display.name);
      rememberFile(path);
      setStatus(`Selected ${fileName(path)}`);
      pushToast({ tone: "info", message: `Selected ${fileName(path)}` });
    },
    [pushToast, rememberFile, transport],
  );

  const spawnChild = useCallback(
    (
      panelId: string,
      kind: ChildKind,
      initialState?: Record<string, unknown>,
      forceNew = false,
      focus = true,
    ) => {
      const api = apiRef.current;
      const parentPanel = api?.getPanel(panelId);
      if (!api || !parentPanel) {
        return;
      }
      const parent = displayFromPanel(parentPanel, nextId.current);
      if (!canSpawnChild(parent.kind, kind)) {
        pushToast({
          tone: "warn",
          message:
            kind === "script_console"
              ? "The Script Console can only be spawned from a Scripting display."
              : "Order and Line Cuts can only be spawned from a Spectrogram.",
        });
        return;
      }

      if (kind === "script_console") {
        const existing = api.panels.find((panel) => {
          const display = (
            panel.params as { display?: ShellDisplay } | undefined
          )?.display;
          return (
            display?.kind === "script_console" && display.parent === parent.id
          );
        });
        if (existing) {
          if (focus) {
            existing.api.setActive();
          }
          return;
        }
      }

      const incomingLines =
        kind === "line_cuts" && initialState
          ? readLines(initialState)
          : [];
      if (!forceNew && incomingLines.length > 0) {
        const existing = api.panels.find((panel) => {
          const display = (
            panel.params as { display?: ShellDisplay } | undefined
          )?.display;
          return (
            display?.kind === "line_cuts" && display.parent === parent.id
          );
        });
        if (existing) {
          const display = (
            existing.params as { display?: ShellDisplay } | undefined
          )?.display;
          if (display) {
            existing.api.updateParameters({
              display: {
                ...display,
                state: {
                  ...display.state,
                  lines: [...readLines(display.state), ...incomingLines],
                },
              },
            });
            existing.api.setActive();
            scheduleSave(api);
            setStatus(`Added line to ${display.name}`);
            pushToast({
              tone: "info",
              message: `Added line to ${display.name}`,
            });
            return;
          }
        }
      }

      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const base = makeChildDisplay(parent, kind, id);
      const child: ShellDisplay = initialState
        ? { ...base, state: { ...base.state, ...initialState } }
        : base;
      const ephemeral = isEphemeralDisplay(child);
      if (ephemeral) {
        suppressLayoutSave.current = true;
      }
      // Keep all children of one Spectrogram in a single companion group.
      const existingChild = api.panels.find((panel) => {
        const display = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        return display?.parent === parent.id;
      });
      const position = existingChild
        ? {
            referencePanel: existingChild,
            direction: "within" as const,
          }
        : {
            referencePanel: parentPanel,
            direction: "below" as const,
          };
      const panel = api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: child.name,
        params: { display: child },
        position,
        ...(kind === "script_console" && !focus ? { inactive: true } : {}),
      });
      if (kind === "script_console" && focus) {
        panel.api.setActive();
      }
      if (ephemeral) {
        queueMicrotask(() => {
          suppressLayoutSave.current = false;
        });
      }
      setStatus(`Opened ${child.name}`);
      pushToast({ tone: "info", message: `Opened ${child.name}` });
    },
    [allocateUserId, pushToast, scheduleSave],
  );

  const lineCutSiblings = useCallback((panelId: string): LineCutSibling[] => {
    const api = apiRef.current;
    if (!api) {
      return [];
    }
    const sourceDisplay = (
      api.getPanel(panelId)?.params as { display?: ShellDisplay } | undefined
    )?.display;
    const parent = sourceDisplay?.parent ?? null;
    const found: Array<LineCutSibling & { sameParent: boolean }> = [];
    for (const panel of api.panels) {
      if (panel.id === panelId) {
        continue;
      }
      const display = (
        panel.params as { display?: ShellDisplay } | undefined
      )?.display;
      if (!display || display.kind !== "line_cuts") {
        continue;
      }
      found.push({
        panelId: panel.id,
        displayId: display.id,
        name: display.name,
        sameParent: display.parent === parent,
      });
    }
    found.sort(
      (a, b) =>
        Number(b.sameParent) - Number(a.sameParent) ||
        a.name.localeCompare(b.name),
    );
    return found.map(({ panelId, displayId, name }) => ({
      panelId,
      displayId,
      name,
    }));
  }, []);

  const moveLineCut = useCallback(
    (fromPanelId: string, lineIndex: number, toPanelId: string | null) => {
      const api = apiRef.current;
      const sourcePanel = api?.getPanel(fromPanelId);
      const sourceDisplay = (
        sourcePanel?.params as { display?: ShellDisplay } | undefined
      )?.display;
      if (
        !api ||
        !sourcePanel ||
        !sourceDisplay ||
        sourceDisplay.kind !== "line_cuts"
      ) {
        return;
      }
      const sourceLines = readLines(sourceDisplay.state);
      const line = sourceLines[lineIndex];
      if (!line) {
        return;
      }
      const remaining = sourceLines.filter((_, index) => index !== lineIndex);

      if (toPanelId === null) {
        const parentPanel = api.panels.find((panel) => {
          const display = (
            panel.params as { display?: ShellDisplay } | undefined
          )?.display;
          return display?.id === sourceDisplay.parent;
        });
        if (!parentPanel) {
          pushToast({
            tone: "warn",
            message: "Cannot create a Line Cuts window without its parent Spectrogram.",
          });
          return;
        }
        sourcePanel.api.updateParameters({
          display: {
            ...sourceDisplay,
            state: { ...sourceDisplay.state, lines: remaining },
          },
        });
        spawnChild(parentPanel.id, "line_cuts", { lines: [line] }, true);
        scheduleSave(api);
        setStatus("Moved line to a new Line Cuts window");
        return;
      }

      if (toPanelId === fromPanelId) {
        return;
      }
      const targetPanel = api.getPanel(toPanelId);
      const targetDisplay = (
        targetPanel?.params as { display?: ShellDisplay } | undefined
      )?.display;
      if (
        !targetPanel ||
        !targetDisplay ||
        targetDisplay.kind !== "line_cuts"
      ) {
        return;
      }
      targetPanel.api.updateParameters({
        display: {
          ...targetDisplay,
          state: {
            ...targetDisplay.state,
            lines: [...readLines(targetDisplay.state), line],
          },
        },
      });
      sourcePanel.api.updateParameters({
        display: {
          ...sourceDisplay,
          state: { ...sourceDisplay.state, lines: remaining },
        },
      });
      targetPanel.api.setActive();
      scheduleSave(api);
      setStatus(`Moved line to ${targetDisplay.name}`);
      pushToast({
        tone: "info",
        message: `Moved line to ${targetDisplay.name}`,
      });
    },
    [pushToast, scheduleSave, spawnChild],
  );

  const updateLineCut = useCallback(
    (panelId: string, lineIndex: number, line: LineSpec) => {
      const api = apiRef.current;
      const panel = api?.getPanel(panelId);
      const display = (
        panel?.params as { display?: ShellDisplay } | undefined
      )?.display;
      if (!api || !panel || display?.kind !== "line_cuts") {
        return;
      }
      const lines = readLines(display.state);
      if (lineIndex < 0 || lineIndex >= lines.length) {
        return;
      }
      const next = [...lines];
      next[lineIndex] = line;
      panel.api.updateParameters({
        display: {
          ...display,
          state: { ...display.state, lines: next },
        },
      });
      scheduleSave(api);
    },
    [scheduleSave],
  );

  const saveSession = useCallback(() => {
    const api = apiRef.current;
    if (api) {
      scheduleSave(api);
    }
  }, [scheduleSave]);

  const savePng = useCallback(
    async (dataUrl: string, filename: string): Promise<string | null> => {
      if (!transport) {
        return null;
      }
      const comma = dataUrl.indexOf(",");
      const dataBase64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
      const result = await transport.invoke({
        name: "image.savePng",
        args: { dataBase64, filename },
      });
      return result?.path ?? null;
    },
    [transport],
  );

  const renamePanel = useCallback((panelId: string, name: string) => {
    const api = apiRef.current;
    const panel = api?.getPanel(panelId);
    if (!panel) {
      return;
    }
    const display = (panel.params as { display?: ShellDisplay } | undefined)
      ?.display;
    if (display) {
      panel.api.updateParameters({
        display: { ...display, name },
      });
    }
    panel.api.setTitle(name);
  }, []);

  const duplicatePanel = useCallback(
    (panelId: string) => {
      const api = apiRef.current;
      const panel = api?.getPanel(panelId);
      if (!api || !panel) {
        return;
      }
      const source = displayFromPanel(panel, nextId.current);
      const id = allocateUserId();
      if (id === null) {
        return;
      }
      const display: ShellDisplay = {
        ...source,
        id,
        name: `${source.name} copy`,
        state: { ...source.state },
      };
      api.addPanel({
        id: `panel-${id}`,
        component: "display",
        title: display.name,
        params: { display },
        position: { referencePanel: panel, direction: "within" },
      });
      setStatus(`Duplicated ${source.name}`);
      pushToast({ tone: "info", message: `Duplicated ${source.name}` });
    },
    [allocateUserId, pushToast],
  );

  const toggleInspector = useCallback((displayId: number) => {
    const api = apiRef.current;
    if (!api) {
      return;
    }
    const inspectorId = `inspector-${displayId}`;
    const existing = api.getPanel(inspectorId);
    if (existing) {
      existing.api.close();
      return;
    }
    const target = api.panels.find((panel) => {
      const display = (panel.params as { display?: ShellDisplay } | undefined)
        ?.display;
      return display?.id === displayId;
    });
    if (!target) {
      return;
    }
    const narrow =
      typeof window !== "undefined" && window.innerWidth < 900;
    const title = target.title ?? "Display";
    api.addPanel({
      id: inspectorId,
      component: "inspector",
      title: `${title} Inspector`,
      params: { displayId },
      position: {
        referencePanel: target,
        direction: narrow ? "within" : "right",
      },
      initialWidth:
        typeof window !== "undefined"
          ? Math.max(300, window.innerWidth * 0.28)
          : 360,
    });
    setStatus(`Inspector opened for ${title}`);
  }, []);

  const requestClose = useCallback((panelId: string) => {
    const api = apiRef.current;
    const panel = api?.getPanel(panelId);
    if (!api || !panel) {
      return;
    }
    const params = panel.params as
      | { display?: ShellDisplay; displayId?: number }
      | undefined;
    const display = params?.display;
    if (display?.kind === "scripting" && scriptingEnabled && transport) {
      void transport
        .invoke({ name: "script.stop", args: { displayId: display.id } })
        .catch(() => {});
    }
    if (
      display &&
      (display.kind === "spectrogram" || display.kind === "scripting")
    ) {
      const displays = api.panels
        .map(
          (candidate) =>
            (candidate.params as { display?: ShellDisplay } | undefined)
              ?.display,
        )
        .filter((value): value is ShellDisplay => Boolean(value));
      const plan = planClose(display, displays);
      if (plan.childCount > 0) {
        pendingClose.current = {
          layout: api.toJSON(),
          panelIds: plan.allIds.map((id) => `panel-${id}`),
          displayIds: plan.allIds,
          childCount: plan.childCount,
          undoable: !isBackendOwnedDisplay(display),
          backendRoot: display.kind === "spectrogram",
        };
        setConfirmClose({
          panelId,
          displayId: display.id,
          childCount: plan.childCount,
        });
        return;
      }
    }
    if (display) {
      if (
        display.kind === "spectrogram" ||
        display.kind === "order_cuts" ||
        display.kind === "line_cuts"
      ) {
        closeDisplayOnBackend(display.id);
      }
      const inspector = api.getPanel(`inspector-${display.id}`);
      if (inspector) {
        api.removePanel(inspector);
      }
    }
    panel.api.close();
  }, [closeDisplayOnBackend, scriptingEnabled, transport]);

  const confirmPendingClose = useCallback(() => {
    const api = apiRef.current;
    const pending = pendingClose.current;
    if (!api || !pending) {
      return;
    }
    const ids = [
      ...pending.panelIds,
      ...pending.displayIds.map((id) => `inspector-${id}`),
    ];
    for (const id of ids) {
      const panel = api.getPanel(id);
      if (panel) {
        api.removePanel(panel);
      }
    }
    const rootId = pending.displayIds[0];
    if (rootId !== undefined && pending.backendRoot) {
      closeDisplayOnBackend(rootId);
    }
    ensureLauncher(api);
    pendingClose.current = null;
    setConfirmClose(null);
    setStatus(`Closed ${pending.displayIds.length} display(s)`);
    if (!pending.undoable) {
      scheduleSave(api);
      pushToast({
        tone: "info",
        message: `Closed ${pending.displayIds.length} display(s)`,
      });
      return;
    }
    undoRef.current = {
      layout: pending.layout,
      displayIds: pending.displayIds,
      createdAt: Date.now(),
    };
    pushToast({
      tone: "info",
      message: `Closed ${pending.displayIds.length} display(s)`,
      actionLabel: "Undo",
      durationMs: UNDO_TTL_MS,
      onAction: () => {
        const current = undoRef.current;
        if (!current || !undoAvailable(current, Date.now())) {
          undoRef.current = null;
          return;
        }
        try {
          apiRef.current?.fromJSON(current.layout as SerializedDockview);
          setStatus("Close undone");
        } catch (error) {
          console.error("could not undo the close", error);
        }
        undoRef.current = null;
      },
    });
  }, [closeDisplayOnBackend, ensureLauncher, pushToast, scheduleSave]);

  const cancelPendingClose = useCallback(() => {
    pendingClose.current = null;
    setConfirmClose(null);
    setStatus("Close cancelled");
  }, []);

  const tabActions = useMemo<TabActions>(
    () => ({
      requestClose,
      rename: renamePanel,
      newLauncher: (panelId) => addLauncherTab(panelId),
      toggleInspector,
      duplicate: duplicatePanel,
    }),
    [
      addLauncherTab,
      duplicatePanel,
      renamePanel,
      requestClose,
      toggleInspector,
    ],
  );

  const openSettings = useCallback(() => {
    const api = apiRef.current;
    if (!api) {
      return;
    }
    const existing = api.getPanel("settings");
    if (existing) {
      existing.api.setActive();
      return;
    }
    api.addPanel({
      id: "settings",
      component: "settings",
      title: "Settings",
    });
    setStatus("Settings opened");
  }, []);

  const exportCuts = useCallback(
    async (cutsToSave: Cut[]): Promise<string | null> => {
      if (!transport) {
        return null;
      }
      const result = await transport.invoke({
        name: "cuts.exportCsv",
        args: { cuts: cutsToSave },
      });
      return result?.path ?? null;
    },
    [transport],
  );

  const displayActions = useMemo<DisplayActions>(
    () => ({
      recentFiles,
      transport,
      scriptingEnabled,
      plotPalette,
      browseForAudio,
      openPath,
      savePng,
      exportCuts,
      spawnChild: (panelId, kind, initialState, options) =>
        spawnChild(panelId, kind, initialState, false, options?.focus),
      lineCutSiblings,
      moveLineCut,
      updateLineCut,
      saveSession,
      toggleInspector,
    }),
    [
      browseForAudio,
      exportCuts,
      savePng,
      lineCutSiblings,
      moveLineCut,
      openPath,
      plotPalette,
      recentFiles,
      saveSession,
      scriptingEnabled,
      spawnChild,
      updateLineCut,
      toggleInspector,
      transport,
    ],
  );

  const closeActive = useCallback(() => {
    const active = apiRef.current?.activePanel;
    if (!active) {
      setStatus("No active tab");
      return;
    }
    requestClose(active.id);
  }, [requestClose]);

  const cycleTab = useCallback((direction: 1 | -1) => {
    const api = apiRef.current;
    if (!api || api.panels.length === 0) {
      return;
    }
    const active = api.activePanel;
    const index = active
      ? api.panels.findIndex((panel) => panel.id === active.id)
      : -1;
    const next =
      (index + direction + api.panels.length) % api.panels.length;
    api.panels[next]?.api.setActive();
  }, []);

  const maximizeActive = useCallback(() => {
    const api = apiRef.current;
    const active = api?.activePanel;
    if (!api || !active) {
      return;
    }
    if (api.hasMaximizedGroup()) {
      api.exitMaximizedGroup();
      setStatus("Restored layout");
    } else {
      api.maximizeGroup(active);
      setStatus(`Maximized ${active.title ?? "tab"}`);
    }
  }, []);

  const activeDisplayId = useCallback((): number | undefined => {
    const active = apiRef.current?.activePanel;
    if (!active) {
      return undefined;
    }
    const params = active.params as
      | { display?: ShellDisplay; displayId?: number }
      | undefined;
    return params?.display?.id ?? params?.displayId;
  }, []);

  const openAudio = useCallback(async () => {
    const path = await browseForAudio();
    if (!path) {
      return;
    }
    addSpectrogram(path);
  }, [addSpectrogram, browseForAudio]);

  const openPlot = useCallback(
    async (layout: PlotLayout) => {
      const path = await browseForAudio();
      if (!path) {
        return;
      }
      addPlotDisplay(path, layout);
    },
    [addPlotDisplay, browseForAudio],
  );

  const resetLayout = useCallback(() => {
    const api = apiRef.current;
    if (!api) {
      return;
    }
    // Best effort: drop live registry records for every display panel before
    // clearing the dock, so Reset layout does not orphan backend analyses/cuts.
    const liveDisplays = api.panels.flatMap((panel) => {
      const display = (
        panel.params as { display?: ShellDisplay } | undefined
      )?.display;
      return display &&
        (display.kind === "spectrogram" ||
          display.kind === "order_cuts" ||
          display.kind === "line_cuts")
        ? [display.id]
        : [];
    });
    if (scriptingEnabled && transport) {
      for (const panel of api.panels) {
        const display = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        if (display?.kind === "scripting") {
          void transport
            .invoke({ name: "script.stop", args: { displayId: display.id } })
            .catch(() => {});
        }
      }
    }
    restoring.current = true;
    api.clear();
    nextId.current = 1;
    pendingDisplayEvents.current = [];
    localCloseIds.current.clear();
    for (const timer of deferredCloses.current.values()) {
      window.clearTimeout(timer);
    }
    deferredCloses.current.clear();
    for (const id of liveDisplays) {
      closeDisplayOnBackend(id);
    }
    addLauncher(api);
    restoring.current = false;
    scheduleSave(api);
    setStatus("Layout reset");
    pushToast({ tone: "success", message: "Layout reset" });
  }, [
    addLauncher,
    closeDisplayOnBackend,
    pushToast,
    scheduleSave,
    scriptingEnabled,
    transport,
  ]);

  const applyPreset = useCallback(
    (name: string) => {
      const api = apiRef.current;
      if (!api) {
        return;
      }
      if (name === "Analysis") {
        const id = activeDisplayId();
        if (id !== undefined && !api.getPanel(`inspector-${id}`)) {
          toggleInspector(id);
        } else {
          setStatus("Analysis layout");
        }
        return;
      }
      if (name === "Cuts side by side") {
        const active = api.activePanel;
        const display = (
          active?.params as { display?: ShellDisplay } | undefined
        )?.display;
        if (active && display?.kind === "spectrogram") {
          const children = api.panels
            .map(
              (panel) =>
                (panel.params as { display?: ShellDisplay } | undefined)
                  ?.display,
            )
            .filter((value): value is ShellDisplay => Boolean(value))
            .filter((value) => value.parent === display.id);
          if (!children.some((child) => child.kind === "order_cuts")) {
            spawnChild(active.id, "order_cuts");
          }
          if (!children.some((child) => child.kind === "line_cuts")) {
            spawnChild(active.id, "line_cuts");
          }
          setStatus("Cuts side by side");
        }
        return;
      }
      if (name === "Focus") {
        if (!api.hasMaximizedGroup()) {
          maximizeActive();
        }
        return;
      }
      const layout = presetsRef.current[name];
      if (layout) {
        try {
          api.fromJSON(layout as SerializedDockview);
          setStatus(`Applied layout: ${name}`);
        } catch (error) {
          pushToast({
            tone: "error",
            message: `Could not apply layout: ${
              error instanceof Error ? error.message : String(error)
            }`,
          });
        }
      }
    },
    [activeDisplayId, maximizeActive, pushToast, spawnChild, toggleInspector],
  );

  const saveCurrentPreset = useCallback(() => {
    const api = apiRef.current;
    if (!api) {
      return;
    }
    const name = uniquePresetName(presetsRef.current, presetName);
    const next = savePreset(presetsRef.current, name, api.toJSON());
    presetsRef.current = next;
    setLayoutPresets(next);
    setPresetDialogOpen(false);
    setPresetName("");
    scheduleSave(api);
    pushToast({ tone: "success", message: `Saved layout preset: ${name}` });
  }, [presetName, pushToast, scheduleSave]);

  const escape = useCallback(() => {
    setPaletteOpen(false);
    setShortcutsOpen(false);
    setPresetDialogOpen(false);
    setConfirmClose(null);
    pendingClose.current = null;
    window.dispatchEvent(new Event("fft:escape"));
  }, []);

  const commands = useMemo<UiCommand[]>(() => {
    const list: UiCommand[] = [
      {
        id: "tab.newLauncher",
        title: "New Launcher",
        category: "Tabs",
        icon: "▤",
        run: () => addLauncherTab(),
      },
      {
        id: "tab.new",
        title: "New Spectrogram",
        category: "Tabs",
        icon: "＋",
        shortcut: "Ctrl+T",
        run: () => spawnSpectrogram(),
      },
      {
        id: "tab.close",
        title: "Close active tab",
        category: "Tabs",
        icon: "×",
        shortcut: "Ctrl+W",
        run: () => closeActive(),
      },
      {
        id: "tab.next",
        title: "Next tab",
        category: "Tabs",
        icon: "→",
        shortcut: "Ctrl+Tab",
        run: () => cycleTab(1),
      },
      {
        id: "tab.prev",
        title: "Previous tab",
        category: "Tabs",
        icon: "←",
        shortcut: "Ctrl+Shift+Tab",
        run: () => cycleTab(-1),
      },
      {
        id: "file.open",
        title: "Open file…",
        category: "File",
        icon: "▤",
        shortcut: "Ctrl+O",
        run: () => openAudio(),
      },
      {
        id: "file.newPlot",
        title: "New Plot…",
        category: "File",
        icon: "∿",
        keywords: ["csv", "x", "y", "chart", "table"],
        enabled: () => transport?.mode !== "browser",
        run: () => openPlot("columns"),
      },
      {
        id: "file.newPlotPairs",
        title: "New Plot (x/y pairs)…",
        category: "File",
        icon: "∿",
        keywords: ["csv", "x", "y", "pairs", "chart", "table"],
        enabled: () => transport?.mode !== "browser",
        run: () => openPlot("pairs"),
      },
      {
        id: "file.exportImage",
        title: "Export image",
        category: "File",
        icon: "⇩",
        enabled: () =>
          activePanelKind === "spectrogram" ||
          activePanelKind === "order_cuts" ||
          activePanelKind === "line_cuts" ||
          activePanelKind === "plot",
        run: () =>
          window.dispatchEvent(
            new CustomEvent("fft:export-image", {
              detail: { panelId: apiRef.current?.activePanel?.id },
            }),
          ),
      },
      {
        id: "view.palette",
        title: "Command palette",
        category: "View",
        icon: "⌘",
        shortcut: "Ctrl+K",
        run: () => setPaletteOpen(true),
      },
      {
        id: "settings.open",
        title: "Settings",
        category: "View",
        icon: "⚙",
        shortcut: "Ctrl+,",
        run: () => openSettings(),
      },
      {
        id: "inspector.toggle",
        title: "Toggle inspector",
        category: "View",
        icon: "ⓘ",
        shortcut: "P",
        enabled: () =>
          activeDisplayId() !== undefined &&
          activePanelKind !== "scripting" &&
          activePanelKind !== "script_console",
        run: () => {
          const id = activeDisplayId();
          if (id !== undefined) {
            toggleInspector(id);
          }
        },
      },
      {
        id: "display.fit",
        title: "Fit view",
        category: "Display",
        icon: "⤢",
        shortcut: "F",
        enabled: () => activePanelKind === "spectrogram",
        run: () =>
          window.dispatchEvent(
            new CustomEvent("fft:fit-view", {
              detail: { panelId: apiRef.current?.activePanel?.id },
            }),
          ),
      },
      {
        id: "cuts.order",
        title: "Order Cuts",
        category: "Cuts",
        icon: "◫",
        enabled: () => activePanelKind === "spectrogram",
        run: () => {
          const panelId = apiRef.current?.activePanel?.id;
          if (panelId) {
            spawnChild(panelId, "order_cuts");
          }
        },
      },
      {
        id: "cuts.line",
        title: "Line Cuts",
        category: "Cuts",
        icon: "／",
        enabled: () => activePanelKind === "spectrogram",
        run: () => {
          const panelId = apiRef.current?.activePanel?.id;
          if (panelId) {
            spawnChild(panelId, "line_cuts");
          }
        },
      },
      {
        id: "view.maximize",
        title: "Maximize / restore",
        category: "View",
        icon: "▣",
        shortcut: "M",
        enabled: () => apiRef.current?.activePanel !== undefined,
        run: () => maximizeActive(),
      },
      {
        id: "layout.reset",
        title: "Reset layout",
        category: "Layout",
        icon: "↺",
        run: () => resetLayout(),
      },
      {
        id: "layout.analysis",
        title: "Layout: Analysis",
        category: "Layout",
        icon: "◫",
        run: () => applyPreset("Analysis"),
      },
      {
        id: "layout.cuts",
        title: "Layout: Cuts side by side",
        category: "Layout",
        icon: "◫",
        run: () => applyPreset("Cuts side by side"),
      },
      {
        id: "layout.focus",
        title: "Layout: Focus",
        category: "Layout",
        icon: "▣",
        run: () => applyPreset("Focus"),
      },
      {
        id: "layout.savePreset",
        title: "Save current layout as preset…",
        category: "Layout",
        icon: "★",
        run: () => setPresetDialogOpen(true),
      },
      {
        id: "help.shortcuts",
        title: "Keyboard shortcuts",
        category: "Help",
        icon: "⌨",
        shortcut: "F1",
        run: () => setShortcutsOpen(true),
      },
      {
        id: "help.about",
        title: "About fft_audio",
        category: "Help",
        icon: "ⓘ",
        run: () => openSettings(),
      },
      {
        id: "escape",
        title: "Cancel / close popup",
        category: "Help",
        icon: "Esc",
        shortcut: "Esc",
        run: () => escape(),
      },
    ];

    for (const name of Object.keys(layoutPresets).sort((a, b) =>
      a.localeCompare(b),
    )) {
      list.push({
        id: `layout.preset.${name}`,
        title: `Layout preset: ${name}`,
        category: "Layout",
        icon: "★",
        run: () => applyPreset(name),
      });
    }
    for (const path of recentRef.current) {
      list.push({
        id: `recent.${path}`,
        title: fileName(path),
        category: "Recent",
        icon: "◷",
        keywords: [path],
        run: () => addSpectrogram(path),
      });
    }
    return list;
  }, [
    activeDisplayId,
    activePanelKind,
    addLauncherTab,
    addSpectrogram,
    applyPreset,
    closeActive,
    cycleTab,
    escape,
    layoutPresets,
    maximizeActive,
    openAudio,
    openPlot,
    openSettings,
    recentFiles,
    resetLayout,
    spawnChild,
    spawnSpectrogram,
    toggleInspector,
    transport,
  ]);

  const registry = useMemo(() => new CommandRegistry(commands), [commands]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing = Boolean(
        target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.isContentEditable),
      );
      if (event.key === "Escape") {
        void registry.run("escape");
        return;
      }
      if (typing && event.key !== "F1") {
        return;
      }
      const command = registry
        .all()
        .find(
          (candidate) =>
            candidate.shortcut &&
            shortcutMatches(event, candidate.shortcut),
        );
      if (command) {
        event.preventDefault();
        void registry.run(command.id);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [registry]);

  useEffect(() => {
    return () => {
      layoutDisposable.current?.dispose();
      activeDisposable.current?.dispose();
      if (saveTimer.current !== undefined) {
        window.clearTimeout(saveTimer.current);
      }
      for (const timer of deferredCloses.current.values()) {
        window.clearTimeout(timer);
      }
      deferredCloses.current.clear();
    };
  }, []);

  const dockTheme = useMemo<DockviewTheme>(
    () => ({
      name: "fft",
      className: "dockview-theme-fft",
      colorScheme: theme.appearance,
      gap: 2,
      dndTabIndicator: "line",
    }),
    [theme.appearance],
  );

  return (
    <div className="shell">
      <TopBar
        appName={appInfo?.name ?? "fft_audio"}
        mode={mode as "desktop" | "headless" | "mock" | "preview"}
        registry={registry}
      />
      <div className="workspace" data-testid="workspace" data-ui-id="shell.workspace">
        <LauncherActionsContext.Provider value={launcherActions}>
          <DisplayActionsContext.Provider value={displayActions}>
            <TabActionsContext.Provider value={tabActions}>
              <DockviewReact
                components={DOCK_COMPONENTS}
                defaultTabComponent={FftTab}
                leftHeaderActionsComponent={AddTabButton}
                getTabContextMenuItems={() => []}
                onReady={onReady}
                theme={dockTheme}
              />
            </TabActionsContext.Provider>
          </DisplayActionsContext.Provider>
        </LauncherActionsContext.Provider>
      </div>
      <StatusBar
        status={status}
        mode={mode}
        version={appInfo?.version ?? "0.12.0"}
        themeName={theme.name}
        notificationCount={unreadCount}
        onOpenNotifications={() => setDrawerOpen(true)}
      />
      <ToastStack />
      <NotificationDrawer
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
      />

      {paletteOpen ? (
        <CommandPalette
          registry={registry}
          onRun={(id) => void registry.run(id)}
          onClose={() => setPaletteOpen(false)}
        />
      ) : null}

      {shortcutsOpen ? (
        <ShortcutsModal onClose={() => setShortcutsOpen(false)} />
      ) : null}

      {presetDialogOpen ? (
        <div className="modal-backdrop" onPointerDown={() => setPresetDialogOpen(false)}>
          <div
            className="preset-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="Save layout preset"
            onPointerDown={(event) => event.stopPropagation()}
          >
            <header>
              <h2>Save layout preset</h2>
            </header>
            <p>Save the current Dockview layout under a name.</p>
            <div style={{ padding: "0 12px 12px" }}>
              <input
                aria-label="Preset name"
                autoFocus
                value={presetName}
                placeholder="My layout"
                onChange={(event) => setPresetName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    saveCurrentPreset();
                  } else if (event.key === "Escape") {
                    setPresetDialogOpen(false);
                  }
                }}
              />
            </div>
            <footer>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setPresetDialogOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={saveCurrentPreset}
              >
                Save preset
              </button>
            </footer>
          </div>
        </div>
      ) : null}

      {confirmClose ? (
        <div className="modal-backdrop" onPointerDown={cancelPendingClose}>
          <div
            className="confirm-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="Close displays"
            onPointerDown={(event) => event.stopPropagation()}
          >
            <header>
              <h2>Close this display?</h2>
            </header>
            <p>
              It has {confirmClose.childCount} child display
              {confirmClose.childCount === 1 ? "" : "s"}. Closing it also closes
              the child displays. You can undo for 8 seconds.
            </p>
            <footer>
              <button
                type="button"
                className="secondary-button"
                onClick={cancelPendingClose}
              >
                Cancel
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={confirmPendingClose}
              >
                Close all
              </button>
            </footer>
          </div>
        </div>
      ) : null}
    </div>
  );
}
