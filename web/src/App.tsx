import { useMemo } from "react";
import { AppShell } from "./shell/AppShell";
import {
  createBrowserTransport,
  isBrowserRuntime,
} from "./transport/browser";
import { createHttpTransport, isHeadlessRuntime } from "./transport/http";
import { createTauriTransport, isTauriRuntime } from "./transport/tauri";
import type { Transport } from "./transport/types";
import { ToastProvider } from "./ui/toast";
import "./ui/ui.css";

/**
 * One UI for both modes: Tauri IPC in the desktop webview, no transport in a
 * plain browser preview (the Dockview shell still renders).
 */
export default function App() {
  const transport = useMemo<Transport | null>(() => {
    if (isTauriRuntime()) {
      return createTauriTransport();
    }
    if (isHeadlessRuntime()) {
      return createHttpTransport();
    }
    if (isBrowserRuntime()) {
      return createBrowserTransport();
    }
    return null;
  }, []);

  return (
    <ToastProvider>
      <AppShell transport={transport} />
    </ToastProvider>
  );
}
