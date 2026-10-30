import { useEffect, type ReactNode } from "react";

/** egui's `Modal`: over the window, closed by Escape or a click outside. `labelledBy` names it for assistive tech. */
export function Modal({
  labelledBy,
  className = "",
  onClose,
  children,
}: {
  labelledBy: string;
  className?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div className="dialog-backdrop" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`dialog ${className}`} role="dialog" aria-modal="true" aria-labelledby={labelledBy}>
        {children}
      </div>
    </div>
  );
}
