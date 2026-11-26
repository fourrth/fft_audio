import { useEffect } from "react";
import { EmptyState } from "./EmptyState";
import { IconButton } from "./IconButton";
import { useToast, type ToastRecord, type ToastTone } from "./toast";

const TONE_LABEL: Record<ToastTone, string> = {
  info: "Info",
  success: "Success",
  warn: "Warning",
  error: "Error",
};

function formatTime(createdAt: number): string {
  return new Date(createdAt).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function NotificationRow({
  toast,
  onAction,
}: {
  toast: ToastRecord;
  onAction?: () => void;
}) {
  return (
    <li className={`notification${toast.read ? " read" : " unread"}`}>
      <span className={`notification-tone tone-${toast.tone}`}>
        {TONE_LABEL[toast.tone]}
      </span>
      <span className="notification-body">
        <span className="notification-message">{toast.message}</span>
        <span className="notification-time">{formatTime(toast.createdAt)}</span>
      </span>
      {toast.actionLabel && onAction ? (
        <button type="button" className="notification-action" onClick={onAction}>
          {toast.actionLabel}
        </button>
      ) : null}
    </li>
  );
}

export interface NotificationDrawerProps {
  open: boolean;
  onClose: () => void;
}

/** Persistent notification log opened from the status-bar bell. */
export function NotificationDrawer({ open, onClose }: NotificationDrawerProps) {
  const {
    notifications,
    unreadCount,
    markAllRead,
    clearNotifications,
  } = useToast();

  useEffect(() => {
    if (open) {
      markAllRead();
    }
  }, [markAllRead, notifications.length, open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  return (
    <div className="drawer-backdrop" onPointerDown={onClose}>
      <aside
        className="notifications-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="Notifications"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header className="drawer-header">
          <div>
            <h2>Notifications</h2>
            <p>
              {notifications.length} recorded
              {unreadCount > 0 ? ` · ${unreadCount} unread` : ""}
            </p>
          </div>
          <div className="drawer-actions">
            <button
              type="button"
              className="text-button"
              onClick={clearNotifications}
              disabled={notifications.length === 0}
            >
              Clear all
            </button>
            <IconButton label="Close notifications" onClick={onClose}>
              ×
            </IconButton>
          </div>
        </header>
        <div className="drawer-body">
          {notifications.length === 0 ? (
            <EmptyState
              icon="🔔"
              title="No notifications yet"
              description="Toasts are recorded here, including errors that stay on screen."
            />
          ) : (
            <ul className="notification-list">
              {notifications.map((toast) => (
                <NotificationRow
                  key={toast.id}
                  toast={toast}
                  onAction={toast.onAction}
                />
              ))}
            </ul>
          )}
        </div>
      </aside>
    </div>
  );
}
