import { useMemo, useState } from "react";
import { settings as tokens } from "../generated/layout";
import type { LogEntry } from "../native/native";
import { clock } from "../shell/Footer";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { VirtualRows } from "../ui/VirtualRows";
import { ONLY_IN_APP } from "./General";
import { ALL_LEVELS, logRows, shown, type Levels, type Source } from "./log";
import { Page } from "../ui/Page";

// A port of crates/fleet-gui/src/views/settings/log.rs's `show`, `controls` and `draw_row`.

const SOURCES: [Source, string][] = [
  ["app", "App"],
  ["daemon", "Daemon"],
  ["both", "Both"],
];
const LEVELS: (keyof Levels)[] = ["error", "warning", "info", "debug"];

interface Props {
  /** The bridge is there: in a browser the files are another machine's. */
  inApp: boolean;
  app: { entries: LogEntry[]; notice: string | null } | null;
  daemon: { entries: LogEntry[]; notice: string | null } | null;
  live: LogEntry[];
}

export function LogTab({ inApp, app, daemon, live }: Props) {
  const [source, setSource] = useState<Source>("both");
  const [levels, setLevels] = useState<Levels>(ALL_LEVELS);
  const [search, setSearch] = useState("");
  const rows = useMemo(
    () => shown(logRows(app?.entries ?? [], live, daemon?.entries ?? [], source), levels, search),
    [app, daemon, live, source, levels, search],
  );
  // One notice per file the source shows.
  const notices = [source !== "daemon" ? app?.notice : null, source !== "app" ? daemon?.notice : null].filter((n): n is string => !!n);
  return (
    <Page className="settings-page log-page">
      <div className="settings-row log-controls">
        {SOURCES.map(([id, label]) => (
          <Chip key={id} selected={source === id} onClick={() => setSource(id)}>
            {label}
          </Chip>
        ))}
        <span className="log-gap" />
        {/* egui's `toggle`: a plain button that stays pressed, not a chip. */}
        {LEVELS.map((l) => (
          <Button key={l} pressed={levels[l]} onClick={() => setLevels({ ...levels, [l]: !levels[l] })}>
            {l}
          </Button>
        ))}
        <span className="log-gap" />
        <input className="log-search" placeholder="Search" aria-label="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
      </div>
      {!inApp && <p className="note">{ONLY_IN_APP}</p>}
      {notices.map((n) => (
        <p key={n} className="note">
          {n}
        </p>
      ))}
      <VirtualRows className="log-rows" count={rows.length} rowHeight={tokens.logRow} row={(i) => <LogRow entry={rows[i]!.entry} />} />
    </Page>
  );
}

/** egui's `draw_row`: the time, the level dot (or its space, for a stray line), then target, message and fields on
 * one line, cut off; the whole line is the hover. */
function LogRow({ entry: e }: { entry: LogEntry }) {
  return (
    <div className="log-row" data-testid="log-row">
      <span className="log-time">{clock(e.time, true)}</span>
      {e.level ? <span className="log-dot" data-level={e.level} /> : <span className="log-dot-space" />}
      <span className="log-text" title={e.line}>
        {e.level && <span className="log-low">{e.target} </span>}
        <span className="log-message" data-level={e.level ?? undefined}>
          {e.message.replaceAll("\n", "\\n")}
        </span>
        {e.fields.map(([k, v]) => (
          <span key={k} className="log-low">{` ${k}=${v.replaceAll("\n", "\\n")}`}</span>
        ))}
      </span>
    </div>
  );
}
