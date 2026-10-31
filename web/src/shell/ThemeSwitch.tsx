import type { Theme } from "./preferences";
import { Glyph } from "../ui/icons";
import { panel } from "../generated/layout";

const THEMES: [Theme, string][] = [
  ["system", "System"],
  ["dark", "Dark"],
  ["light", "Light"],
];

/** The top bar's theme picker: one icon per theme, the current one pressed. */
export function ThemeSwitch({ current, onPick }: { current: Theme; onPick: (theme: Theme) => void }) {
  return (
    <div className="theme-switch" role="group" aria-label="Theme">
      {THEMES.map(([theme, label]) => (
        <button
          key={theme}
          type="button"
          className="icon-button"
          data-size="header"
          aria-label={`${label} theme`}
          title={`${label} theme`}
          aria-pressed={theme === current}
          onClick={() => onPick(theme)}
        >
          <Glyph name={theme} size={panel.iconButton} />
        </button>
      ))}
    </div>
  );
}
