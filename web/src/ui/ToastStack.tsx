import { useToast, type ToastRecord, type ToastTone } from "./toast";

const ICONS: Record<ToastTone, string> = {
  info: "i",
  success: "✓",
  warn: "!",
  error: "×",
};

export interface ToastItemProps {
  toast: ToastRecord;
  onDismiss: () => void;
  onAction?: () => void;
}

export function ToastItem({ toast, onDismiss, onAction }: ToastItemProps) {
  return (
    <div
      className={`toast toast-${toast.tone}`}
      role={toast.tone === "error" ? "alert" : "status"}
    >
      <span className="toast-icon" aria-hidden="true">
        {ICONS[toast.tone]}
      </span>
      <span className="toast-message">{toast.message}</span>
      {toast.actionLabel && onAction ? (
        <button type="button" className="toast-action" onClick={onAction}>
          {toast.actionLabel}
        </button>
      ) : null}
      <button
        type="button"
        className="toast-close"
        aria-label="Dismiss notification"
        onClick={onDismiss}
      >
        ×
      </button>
    </div>
  );
}

/** Bottom-right stack; max 3 visible, newest at the bottom. */
export function ToastStack() {
  const { toasts, dismissToast } = useToast();
  if (toasts.length === 0) {
    return null;
  }
  return (
    <div
      className="toast-stack"
      aria-live="polite"
      aria-relevant="additions"
      data-testid="toast-stack"
    >
      {toasts.map((toast) => (
        <ToastItem
          key={toast.id}
          toast={toast}
          onDismiss={() => dismissToast(toast.id)}
          onAction={
            toast.onAction
              ? () => {
                  toast.onAction?.();
                  dismissToast(toast.id);
                }
              : undefined
          }
        />
      ))}
    </div>
  );
}
