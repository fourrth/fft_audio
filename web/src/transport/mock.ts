import type {
  AppMode,
  Command,
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  Transport,
  Unsubscribe,
} from "./types";

export type MockHandler<K extends CommandName> = (
  args: CommandMap[K]["args"],
) => CommandMap[K]["result"] | Promise<CommandMap[K]["result"]>;

export type MockHandlers = Partial<{
  [K in CommandName]: MockHandler<K>;
}>;

type StoredHandler = (args: never) => unknown;
type StoredListener = (payload: never) => void;

/**
 * In-memory Transport used by tests and by the UI while no real backend is attached.
 */
export class MockTransport implements Transport {
  readonly mode: AppMode = "mock";
  readonly calls: Command[] = [];

  private readonly handlers = new Map<CommandName, StoredHandler>();
  private readonly listeners = new Map<EventName, Set<StoredListener>>();

  constructor(handlers: MockHandlers = {}) {
    for (const name of Object.keys(handlers) as CommandName[]) {
      const handler = handlers[name];
      if (handler) {
        this.handlers.set(name, handler as StoredHandler);
      }
    }
  }

  setHandler<K extends CommandName>(name: K, handler: MockHandler<K>): void {
    this.handlers.set(name, handler as StoredHandler);
  }

  async invoke<K extends CommandName>(
    command: Command<K>,
  ): Promise<CommandMap[K]["result"]> {
    this.calls.push(command as Command);

    const handler = this.handlers.get(command.name);
    if (!handler) {
      throw new Error(
        `MockTransport: no handler registered for command "${command.name}"`,
      );
    }

    return (await handler(command.args as never)) as CommandMap[K]["result"];
  }

  on<K extends EventName>(
    event: K,
    listener: (payload: EventMap[K]) => void,
  ): Unsubscribe {
    let listeners = this.listeners.get(event);
    if (!listeners) {
      listeners = new Set();
      this.listeners.set(event, listeners);
    }

    const stored = listener as StoredListener;
    listeners.add(stored);

    return () => {
      listeners.delete(stored);
    };
  }

  emit<K extends EventName>(event: K, payload: EventMap[K]): void {
    const listeners = this.listeners.get(event);
    if (!listeners) {
      return;
    }

    for (const listener of [...listeners]) {
      (listener as (payload: EventMap[K]) => void)(payload);
    }
  }
}
