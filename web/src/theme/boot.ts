import type { Transport } from "../transport/types";
import { applyTheme, type StyleTarget } from "./css";
import type { ResolvedTheme } from "./types";

/**
 * Fetch a resolved TOML theme through the Transport and inject its CSS
 * custom properties. Desktop (Tauri IPC) and headless (HTTP) share this.
 */
export async function loadAndApplyTheme(
  transport: Transport,
  name?: string,
  target?: StyleTarget,
): Promise<ResolvedTheme> {
  const theme = await transport.invoke({
    name: "theme.get",
    args: name ? { name } : {},
  });
  return applyTheme(theme, target);
}
