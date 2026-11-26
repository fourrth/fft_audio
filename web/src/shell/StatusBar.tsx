export interface StatusBarProps {
  status: string;
  mode: string;
  version: string;
  themeName: string;
  notificationCount?: number;
  onOpenNotifications?: () => void;
}

/** 24 px status strip: transient status on the left, app/theme info right. */
export function StatusBar({
  status,
  mode,
  version,
  themeName,
  notificationCount = 0,
  onOpenNotifications,
}: StatusBarProps) {
  return (
    <footer className="statusbar" data-testid="statusbar" data-ui-id="shell.statusbar">
      <div className="status-group" data-ui-id="shell.statusbar.status">
        <span className="status-dot" aria-hidden="true" />
        <span role="status">{status}</span>
      </div>
      <div className="status-group" data-ui-id="shell.statusbar.meta">
        <span className="status-meta">
          {themeName} · {mode} · v{version}
        </span>
        <button
          type="button"
          className="btn-icon bell"
          aria-label={`Notifications (${notificationCount})`}
          onClick={onOpenNotifications}
          disabled={!onOpenNotifications}
          title="Notifications"
        >
          🔔
          {notificationCount > 0 ? (
            <span className="badge">{notificationCount}</span>
          ) : null}
        </button>
      </div>
    </footer>
  );
}
