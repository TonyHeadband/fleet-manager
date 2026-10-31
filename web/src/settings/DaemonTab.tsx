import { useEffect, useState } from "react";
import type { DaemonInfo, Native } from "../native/native";
import { Button } from "../ui/Button";
import { Section } from "../ui/Section";
import { ONLY_IN_APP } from "./General";
import type { RestartApi } from "./restart";

// A port of crates/fleet-gui/src/views/settings/daemon.rs: where the daemon listens, who started it, its paths and
// PID, and Restart daemon.

/** egui re-reads the pidfile on every frame the tab is drawn; once a second is the page's frame for it. */
const EVERY_MS = 1000;

export function DaemonTab({ native, restart }: { native: Native | null; restart: RestartApi }) {
  const [info, setInfo] = useState<DaemonInfo | null>(null);
  useEffect(() => {
    if (!native) return;
    const read = () => void native.daemonInfo().then(setInfo, () => {});
    read();
    const timer = setInterval(read, EVERY_MS);
    return () => clearInterval(timer);
  }, [native]);

  const rows: [string, string | null, boolean][] = info
    ? [
        ["Address", info.address, true],
        ["Started by", info.startedBy, false],
        ["State directory", info.stateDir, true],
        ["Config file", info.configFile, true],
        ["PID", info.pid, true],
      ]
    : [];
  const title = native ? (restart.blocker ?? undefined) : ONLY_IN_APP;
  return (
    <div className="tabbed-page settings-page">
      <Section title="Daemon">
        {!native && <p className="note">{ONLY_IN_APP}</p>}
        <dl className="daemon-rows">
          {rows
            .filter(([, value]) => value !== null)
            .map(([name, value, mono]) => (
              <div key={name}>
                <dt>{name}</dt>
                <dd className={mono ? "mono" : undefined}>{value}</dd>
              </div>
            ))}
        </dl>
        <div className="settings-row daemon-restart">
          <Button disabled={!native || restart.blocker !== null} title={title} onClick={() => restart.ask({ kind: "restart" })}>
            Restart daemon
          </Button>
          {restart.restarting !== null && <span className="note">Restarting the daemon…</span>}
        </div>
      </Section>
    </div>
  );
}
