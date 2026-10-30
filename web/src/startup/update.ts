import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type { Native, UpdateCheck } from "../native/native";

/** egui's update check: at start and once a day. */
export const UPDATE_EVERY_MS = 24 * 3600 * 1000;

const UNKNOWN: UpdateCheck = { kind: "unknown" };

interface Store {
  value: UpdateCheck;
  listeners: Set<() => void>;
  timer?: ReturnType<typeof setInterval>;
}

// One check per bridge, however many read it: the new-version notice and Settings' Version line show the same answer.
const stores = new WeakMap<Native, Store>();

function storeOf(native: Native): Store {
  let s = stores.get(native);
  if (!s) stores.set(native, (s = { value: UNKNOWN, listeners: new Set() }));
  return s;
}

function subscribe(native: Native, listener: () => void) {
  const s = storeOf(native);
  s.listeners.add(listener);
  if (s.listeners.size === 1) {
    // A failed call is not a failed check: the notice stays away and Settings keeps what it showed.
    const check = () =>
      void native.checkUpdate().then(
        (value) => {
          // A new object per answer, even for the same release: the notice tells answers apart by it.
          s.value = { ...value };
          for (const l of s.listeners) l();
        },
        () => {},
      );
    check();
    s.timer = setInterval(check, UPDATE_EVERY_MS);
  }
  return () => {
    s.listeners.delete(listener);
    if (s.listeners.size === 0) clearInterval(s.timer);
  };
}

/** What the release check says; `unknown` in a browser, which has no check. */
export function useUpdateCheck(native: Native | null): UpdateCheck {
  // Stable per bridge: a new subscribe function would resubscribe, and so check again, on every render.
  const listen = useCallback((l: () => void) => (native ? subscribe(native, l) : () => {}), [native]);
  return useSyncExternalStore(listen, () => (native ? storeOf(native).value : UNKNOWN));
}

/** This app's version, as the bridge says; "" until it answers or in a browser. */
export function useAppVersion(native: Native | null): string {
  const [version, setVersion] = useState("");
  useEffect(() => {
    native?.version().then(setVersion, () => {});
  }, [native]);
  return version;
}
