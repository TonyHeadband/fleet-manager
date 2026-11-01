import { useCallback, useEffect, useRef, useState } from "react";
import { navigate } from "../shell/routes";

export interface ToastItem {
  id: number;
  title: string;
  text: string;
  tone: "accent" | "danger" | "done";
  href?: string;
}

export const TOAST_MAX = 3;
export const TOAST_MS = 8_000;

/** A short stack of toasts: the newest {@link TOAST_MAX} stay, each leaves by itself after {@link TOAST_MS}. */
export function useToasts() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const next = useRef(0);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());
  const dismiss = useCallback((id: number) => {
    clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setToasts((all) => all.filter((t) => t.id !== id));
  }, []);
  const push = useCallback(
    (t: Omit<ToastItem, "id">) => {
      const id = next.current++;
      timers.current.set(id, setTimeout(() => dismiss(id), TOAST_MS));
      setToasts((all) => {
        const kept = [...all, { ...t, id }];
        // The oldest make room; their timers must not outlive them.
        for (const old of kept.slice(0, Math.max(0, kept.length - TOAST_MAX))) {
          clearTimeout(timers.current.get(old.id));
          timers.current.delete(old.id);
        }
        return kept.slice(-TOAST_MAX);
      });
    },
    [dismiss],
  );
  useEffect(() => {
    const live = timers.current;
    return () => live.forEach(clearTimeout);
  }, []);
  return { toasts, push, dismiss };
}

export function ToastStack({ toasts, onDismiss }: { toasts: ToastItem[]; onDismiss: (id: number) => void }) {
  // Mounted while empty: a live region that appears together with its first toast is not announced.
  return (
    <div className="toast-stack" role="status">
      {toasts.map((t) => (
        <button
          key={t.id}
          type="button"
          className="toast"
          onClick={() => {
            onDismiss(t.id);
            if (t.href) navigate(t.href);
          }}
        >
          <span className="toast-mark" data-tone={t.tone} aria-hidden="true" />
          <span className="toast-title">{t.title}</span>
          <span className="toast-text">{t.text}</span>
        </button>
      ))}
    </div>
  );
}
