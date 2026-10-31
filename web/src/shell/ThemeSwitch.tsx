import type { Theme } from "./preferences";
import { Glyph } from "../ui/icons";

const THEMES: [Theme, string][] = [
  ["system", "System"],
  ["dark", "Dark"],
  ["light", "Light"],
];

/** The drawer's theme picker, beside the app name: one icon per theme, the current one pressed. */
export function ThemeSwitch({ current, onPick }: { current: Theme; onPick: (theme: Theme) => void }) {
  return (
    <div className="theme-switch" role="group" aria-label="Theme">
      {THEMES.map(([theme, label]) => (
        <button
          key={theme}
          type="button"
          className="icon-button"
          aria-label={`${label} theme`}
          title={`${label} theme`}
          aria-pressed={theme === current}
          onClick={() => onPick(theme)}
        >
          <Glyph name={theme} />
        </button>
      ))}
    </div>
  );
}
