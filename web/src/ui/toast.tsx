import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

export type ToastTone = "info" | "success" | "warn" | "error";

export interface ToastInput {
  tone: ToastTone;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  /** Defaults to 4 s; errors always stay until dismissed. */
  durationMs?: number;
}

export interface ToastRecord extends ToastInput {
  id: number;
  createdAt: number;
  read: boolean;
}

interface ToastContextValue {
  pushToast: (toast: ToastInput) => number;
  dismissToast: (id: number) => void;
  toasts: ToastRecord[];
  notifications: ToastRecord[];
  unreadCount: number;
  markAllRead: () => void;
  clearNotifications: () => void;
}

const noop = () => {};

const ToastContext = createContext<ToastContextValue>({
  pushToast: () => 0,
  dismissToast: noop,
  toasts: [],
  notifications: [],
  unreadCount: 0,
  markAllRead: noop,
  clearNotifications: noop,
});

export function useToast(): ToastContextValue {
  return useContext(ToastContext);
}

export interface ToastProviderProps {
  children: ReactNode;
  maxVisible?: number;
  defaultDurationMs?: number;
}

/**
 * Toast stack + persistent notification log.
 *
 * At most `maxVisible` toasts are on screen; every toast is retained in
 * `notifications` for the drawer. Errors ignore the auto-dismiss timer.
 */
export function ToastProvider({
  children,
  maxVisible = 3,
  defaultDurationMs = 4000,
}: ToastProviderProps) {
  const [toasts, setToasts] = useState<ToastRecord[]>([]);
  const [notifications, setNotifications] = useState<ToastRecord[]>([]);
  const timers = useRef(new Map<number, number>());
  const nextId = useRef(1);

  const dismissToast = useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timers.current.delete(id);
    }
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const pushToast = useCallback(
    (input: ToastInput) => {
      const id = nextId.current++;
      const record: ToastRecord = {
        ...input,
        id,
        createdAt: Date.now(),
        read: false,
      };
      setNotifications((current) => [record, ...current].slice(0, 200));
      setToasts((current) => [...current, record].slice(-maxVisible));

      if (input.tone !== "error") {
        const timer = window.setTimeout(
          () => dismissToast(id),
          input.durationMs ?? defaultDurationMs,
        );
        timers.current.set(id, timer);
      }
      return id;
    },
    [defaultDurationMs, dismissToast, maxVisible],
  );

  const markAllRead = useCallback(() => {
    setNotifications((current) =>
      current.map((toast) => (toast.read ? toast : { ...toast, read: true })),
    );
  }, []);

  const clearNotifications = useCallback(() => {
    setNotifications([]);
  }, []);

  useEffect(() => {
    const active = timers.current;
    return () => {
      active.forEach((timer) => window.clearTimeout(timer));
      active.clear();
    };
  }, []);

  const unreadCount = notifications.reduce(
    (count, toast) => count + (toast.read ? 0 : 1),
    0,
  );

  return (
    <ToastContext.Provider
      value={{
        pushToast,
        dismissToast,
        toasts,
        notifications,
        unreadCount,
        markAllRead,
        clearNotifications,
      }}
    >
      {children}
    </ToastContext.Provider>
  );
}
