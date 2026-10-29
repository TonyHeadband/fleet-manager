import type { ButtonHTMLAttributes } from "react";

export type ButtonVariant = "default" | "primary" | "danger" | "quiet-danger";

interface Props extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  /** Renders a toggle: sets `aria-pressed`. */
  pressed?: boolean;
}

/** The one button. Looks come from the stylesheet, keyed on `data-variant` and `aria-pressed`. */
export function Button({ variant = "default", pressed, type = "button", ...rest }: Props) {
  return <button type={type} data-variant={variant === "default" ? undefined : variant} aria-pressed={pressed} {...rest} />;
}
