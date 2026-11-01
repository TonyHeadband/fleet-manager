import { fleet as fleetSize } from "../generated/layout";

/**
 * The accent plus that turns into a minus when open (its upright turns flat onto its bar), drawn once for the card's
 * fleet fold and the chat's sections.
 */
export function PlusMinus({ open }: { open: boolean }) {
  const s = fleetSize.foldText;
  const c = s / 2;
  return (
    <svg className="plus-minus" data-open={open || undefined} width={s} height={s} viewBox={`0 0 ${s} ${s}`} aria-hidden="true">
      <rect x={1} y={c - 0.75} width={s - 2} height={1.5} />
      <rect className="fold-bar" x={c - 0.75} y={1} width={1.5} height={s - 2} />
    </svg>
  );
}
