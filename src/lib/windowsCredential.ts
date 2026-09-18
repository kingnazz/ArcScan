// The Windows credential's status, shared between the places that show it.
//
// # Why this exists
//
// Two components care whether a credential is set: Settings, which sets and
// clears it, and the command bar's depth picker, which warns when the
// credentialed level is chosen without one. v1.9.0 had the command bar read the
// status once on mount, so setting a credential in Settings left the picker
// still saying none was set — and clearing one left it still saying there was.
// The warning was wrong in both directions for the rest of the session.
//
// The status is not component state. It belongs to the process, it changes from
// one place and is read from another, so it lives here and its readers are
// told when it changes.
//
// # What this is not
//
// It holds the *status* — whether a credential is configured, which account,
// whether this build supports one. It never holds a password: there is no
// command that reads one back, so there is nothing here to cache.

import { api } from "./api";
import type { WindowsCredentialStatus } from "../types";

export type CredentialListener = (status: WindowsCredentialStatus | null) => void;

/** The operations the store needs, injected so it can be tested without IPC. */
export interface CredentialOperations {
  status: () => Promise<WindowsCredentialStatus>;
  set: (
    username: string,
    domain: string | null,
    password: string,
  ) => Promise<WindowsCredentialStatus>;
  clear: () => Promise<WindowsCredentialStatus>;
}

export interface CredentialStore {
  /** The last known status, or null before the first successful read. */
  current(): WindowsCredentialStatus | null;
  /** Re-read from the backend and notify listeners. */
  refresh(): Promise<WindowsCredentialStatus | null>;
  set(
    username: string,
    domain: string | null,
    password: string,
  ): Promise<WindowsCredentialStatus>;
  clear(): Promise<WindowsCredentialStatus>;
  /** Subscribe to changes. Returns an unsubscribe function. */
  subscribe(listener: CredentialListener): () => void;
}

export function createCredentialStore(operations: CredentialOperations): CredentialStore {
  let current: WindowsCredentialStatus | null = null;
  const listeners = new Set<CredentialListener>();

  const publish = (status: WindowsCredentialStatus | null) => {
    current = status;
    // A listener that throws must not stop the others being told, or one bad
    // component would silently freeze every other reader's view.
    for (const listener of [...listeners]) {
      try {
        listener(status);
      } catch {
        // Ignored on purpose; see above.
      }
    }
  };

  return {
    current: () => current,

    async refresh() {
      try {
        const status = await operations.status();
        publish(status);
        return status;
      } catch {
        // A backend that cannot answer is not a credential that is set. The
        // depth picker's warning is the safe reading either way, and throwing
        // here would take out whatever component asked.
        return current;
      }
    },

    async set(username, domain, password) {
      const status = await operations.set(username, domain, password);
      publish(status);
      return status;
    },

    async clear() {
      const status = await operations.clear();
      publish(status);
      return status;
    },

    subscribe(listener) {
      listeners.add(listener);
      // Told immediately, so a component that mounts after the first read does
      // not have to wait for the next change to show anything.
      if (current) listener(current);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/**
 * The store the application uses, wired to the real IPC commands.
 *
 * A module-level singleton because the credential it describes is itself
 * process-wide: one ArcScan has one credential, so one status describes it.
 */
export const windowsCredentials: CredentialStore = createCredentialStore({
  status: () => api.windowsCredentialStatus(),
  set: (username, domain, password) => api.setWindowsCredential(username, domain, password),
  clear: () => api.clearWindowsCredential(),
});
