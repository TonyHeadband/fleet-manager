import { useId, useMemo } from "react";
import { AVATAR_H, AVATAR_W, FIGURE, avatar, type AvatarKind, type Ink } from "./avatarArt";

const paint = (ink: Ink, hue: number) => ({ stroke: `oklch(var(${ink === "main" ? "--avatar-l" : "--avatar-l2"}) var(--avatar-c) ${hue})` });
// Traces turn at right angles, so square caps and miter joins; scan runs keep butt caps so their short gaps stay open.
const TRACE = { strokeLinecap: "square", strokeLinejoin: "miter", fill: "none" } as const;

/**
 * An avatar in the app's portrait shape, 3:4: a picture generated from `seed` (a persona's figure or a specialist's
 * traces) over the sunken frame, or the empty frame without a seed. Each place sizes it; `title` names whose it is.
 */
export function Avatar({ title, kind = "persona", seed }: { title?: string; kind?: AvatarKind; seed?: string | null }) {
  const art = useMemo(() => (seed ? avatar(kind, seed) : null), [kind, seed]);
  // Unique per instance: the same persona is drawn on many cards at once, and a shared id clips one to another.
  // useId's characters (colons, guillemets) are not all safe inside url(#…), so keep only the word characters.
  const id = `avatar${useId().replace(/\W/g, "")}`;
  if (!art) return <span className="avatar" title={title} aria-hidden="true" />;
  const frame = `${id}-frame`,
    figure = `${id}-figure`;
  const { head, shoulders } = FIGURE;
  return (
    <span className="avatar" title={title} aria-hidden="true" data-kind={kind} data-seed={seed ?? undefined}>
      <svg viewBox={`0 0 ${AVATAR_W} ${AVATAR_H}`}>
        <defs>
          <clipPath id={frame}>
            <rect width={AVATAR_W} height={AVATAR_H} />
          </clipPath>
          {kind === "persona" && (
            <clipPath id={figure}>
              <circle cx={head.cx} cy={head.cy} r={head.r} />
              <circle cx={shoulders.cx} cy={shoulders.cy} r={shoulders.r} />
            </clipPath>
          )}
        </defs>
        <g clipPath={`url(#${frame})`}>
          <g clipPath={kind === "persona" ? `url(#${figure})` : undefined}>
            {art.shapes.map((s, i) =>
              s.type === "line" ? (
                <line key={i} x1={s.x1} y1={s.y1} x2={s.x2} y2={s.y2} strokeWidth={s.width} opacity={s.opacity} style={paint(s.ink, art.hue)} />
              ) : (
                <polyline key={i} points={s.points.join(" ")} strokeWidth={s.width} opacity={s.opacity} style={paint(s.ink, art.hue)} {...TRACE} />
              ),
            )}
          </g>
          {kind === "persona" &&
            [head, shoulders].map((c, i) => (
              <circle key={i} cx={c.cx} cy={c.cy} r={c.r} strokeWidth={FIGURE.outline} style={paint("main", art.hue)} fill="none" />
            ))}
        </g>
      </svg>
    </span>
  );
}
