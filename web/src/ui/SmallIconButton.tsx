import { panel } from "../generated/layout";
import { Glyph, type IconName } from "./icons";

interface Props {
  icon: IconName;
  /** The accessible name, and the tooltip unless `title` says otherwise. */
  label: string;
  /** Armed: red on its tint, as Lite's `small_icon_button(.., true)`. */
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  /** A header row's larger button (Lite's `icon_button`), not a card's. */
  large?: boolean;
  onClick: () => void;
}

/** Lite's `small_icon_button`: a square glyph button, quiet until hovered. */
export function SmallIconButton({ icon, label, danger, disabled, title, large, onClick }: Props) {
  return (
    <button
      type="button"
      className="icon-button"
      data-size={large ? "header" : undefined}
      aria-label={label}
      title={title ?? label}
      data-danger={danger || undefined}
      disabled={disabled}
      onClick={(e) => {
        // The card behind it opens on a click; an action must not.
        e.stopPropagation();
        onClick();
      }}
    >
      <Glyph name={icon} size={large ? panel.iconButton : undefined} />
    </button>
  );
}
