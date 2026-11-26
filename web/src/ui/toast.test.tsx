// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { NotificationDrawer } from "./NotificationDrawer";
import { ToastStack } from "./ToastStack";
import { ToastProvider, useToast } from "./toast";

function Harness({ onAction }: { onAction?: () => void }) {
  const { pushToast, unreadCount, notifications } = useToast();
  const [drawerOpen, setDrawerOpen] = useState(false);
  return (
    <div>
      <button
        type="button"
        onClick={() => pushToast({ tone: "info", message: "Saved" })}
      >
        info
      </button>
      <button
        type="button"
        onClick={() => pushToast({ tone: "error", message: "Broken" })}
      >
        error
      </button>
      <button
        type="button"
        onClick={() =>
          pushToast({
            tone: "warn",
            message: "Cut deleted",
            actionLabel: "Undo",
            onAction,
          })
        }
      >
        action
      </button>
      <button
        type="button"
        onClick={() => {
          for (let i = 0; i < 5; i += 1) {
            pushToast({ tone: "success", message: `Toast ${i}` });
          }
        }}
      >
        spam
      </button>
      <button type="button" onClick={() => setDrawerOpen(true)}>
        open drawer
      </button>
      <span data-testid="unread">{unreadCount}</span>
      <span data-testid="recorded">{notifications.length}</span>
      <ToastStack />
      <NotificationDrawer
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
      />
    </div>
  );
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("toast stack and notification drawer", () => {
  it("auto-dismisses non-errors after 4 s but keeps them recorded", () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "info" }));
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(screen.getByTestId("unread").textContent).toBe("1");

    act(() => vi.advanceTimersByTime(4000));
    expect(screen.queryByText("Saved")).toBeNull();
    expect(screen.getByTestId("recorded").textContent).toBe("1");

    fireEvent.click(screen.getByRole("button", { name: "open drawer" }));
    expect(screen.getByRole("dialog", { name: "Notifications" })).toBeTruthy();
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(screen.getByTestId("unread").textContent).toBe("0");
  });

  it("keeps errors until dismissed", () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "error" }));
    act(() => vi.advanceTimersByTime(30_000));
    expect(screen.getByText("Broken")).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "Dismiss notification" }),
    );
    expect(screen.queryByText("Broken")).toBeNull();
  });

  it("runs an optional action and dismisses the toast", () => {
    const onAction = vi.fn();
    render(
      <ToastProvider>
        <Harness onAction={onAction} />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "action" }));
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(onAction).toHaveBeenCalledOnce();
    expect(screen.queryByText("Cut deleted")).toBeNull();
  });

  it("shows at most three toasts but records every one", () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "spam" }));
    expect(screen.getByText("Toast 2")).toBeTruthy();
    expect(screen.getByText("Toast 3")).toBeTruthy();
    expect(screen.getByText("Toast 4")).toBeTruthy();
    expect(screen.queryByText("Toast 0")).toBeNull();
    expect(screen.getByTestId("recorded").textContent).toBe("5");

    fireEvent.click(screen.getByRole("button", { name: "open drawer" }));
    expect(screen.getByText("Toast 0")).toBeTruthy();
  });

  it("marks notifications read and clears the log", () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "info" }));
    fireEvent.click(screen.getByRole("button", { name: "error" }));
    expect(screen.getByTestId("unread").textContent).toBe("2");

    fireEvent.click(screen.getByRole("button", { name: "open drawer" }));
    expect(screen.getByTestId("unread").textContent).toBe("0");
    fireEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(screen.getByText("No notifications yet")).toBeTruthy();
    expect(screen.getByTestId("recorded").textContent).toBe("0");
  });

  it("closes the drawer from its close button", () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "open drawer" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Close notifications" }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
