import { useEffect, useState, type ReactNode } from "react";
import type { FleetClient } from "../api/client";
import type { ToolList } from "../api/generated/ToolList";
import type { Native } from "../native/native";
import { RadarMark } from "../shell/RadarMark";
import { Button } from "../ui/Button";
import { checkingLabel, RETRY_MS, ROW_MS, STARTING, TOOL_ROWS, type IntroProgress } from "./boot";
import "./startup.css";

/** What startup learned, for the board: the daemon's client, whether this window started it, and the tools. */
export interface Started {
  client: FleetClient;
  weStarted: boolean;
  tools: ToolList | null;
}

type Stage =
  | { kind: "starting"; attempt: number }
  | { kind: "failed"; message: string }
  /** `answer` is undefined until the tools check answers, null when it failed. */
  | { kind: "checking"; up: Omit<Started, "tools">; answer?: ToolList | null; shown: number }
  | { kind: "running"; started: Started };

interface Props {
  /** The daemon's client at `url`: in the app the address is known only once the daemon is up (port 0). */
  connect: (url: string) => FleetClient;
  /** The browser's daemon: the one that served the page, unless overridden. */
  browserUrl: string;
  native: Native | null;
  /** Off: the logo is still and the tool rows are not stepped through, as egui with animations off. */
  animate: boolean;
  children: (started: Started) => ReactNode;
}

/**
 * The window until the board: egui's intro while the daemon comes up and answers about the tools, or its failure
 * screen with Retry. In a browser there is no daemon to start, so the intro waits until the daemon answers.
 */
export function Startup({ connect, browserUrl, native, animate, children }: Props) {
  const [stage, setStage] = useState<Stage>({ kind: "starting", attempt: 0 });
  const [stateDir, setStateDir] = useState("");
  const attempt = stage.kind === "starting" ? stage.attempt : -1;

  useEffect(() => {
    if (attempt < 0) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const checking = (client: FleetClient, weStarted: boolean) =>
      live && setStage({ kind: "checking", up: { client, weStarted }, shown: 0 });
    if (native) {
      native.startDaemon().then(
        (r) => checking(connect(r.url), r.weStarted),
        (e: unknown) => live && setStage({ kind: "failed", message: String(e) }),
      );
    } else {
      const client = connect(browserUrl);
      const ask = () =>
        client.uiSchema().then(
          () => checking(client, false),
          () => {
            if (live) timer = setTimeout(ask, RETRY_MS);
          },
        );
      ask();
    }
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [attempt, connect, browserUrl, native]);

  useEffect(() => {
    if (stage.kind === "failed") native?.stateDir().then(setStateDir, () => setStateDir(""));
  }, [stage.kind, native]);

  const checkingWith = stage.kind === "checking" ? stage.up.client : null;
  useEffect(() => {
    if (!checkingWith) return;
    let live = true;
    const settle = (answer: ToolList | null) => live && setStage((s) => (s.kind === "checking" ? { ...s, answer } : s));
    // An unanswered check never blocks the board: egui moves on with no tool list.
    checkingWith.tools().then(settle, () => settle(null));
    return () => {
      live = false;
    };
  }, [checkingWith]);

  const answer = stage.kind === "checking" ? stage.answer : undefined;
  const stepping = animate && answer != null && answer.tools.length > 0;
  useEffect(() => {
    if (answer === undefined) return;
    const run = (s: Stage): Stage => (s.kind === "checking" ? { kind: "running", started: { ...s.up, tools: s.answer ?? null } } : s);
    if (!stepping) {
      setStage(run);
      return;
    }
    const timer = setInterval(
      () =>
        setStage((s) => {
          if (s.kind !== "checking" || !s.answer) return s;
          return s.shown + 1 >= s.answer.tools.length ? run(s) : { ...s, shown: s.shown + 1 };
        }),
      ROW_MS,
    );
    return () => clearInterval(timer);
  }, [answer, stepping]);

  switch (stage.kind) {
    case "running":
      return <>{children(stage.started)}</>;
    case "failed":
      return (
        <main className="startup">
          <section className="startup-card">
            <h2>Fleet Manager could not start its daemon</h2>
            <p className="reason">{stage.message}</p>
            <p className="where">State directory: {stateDir}</p>
            <div className="actions">
              <Button variant="primary" onClick={() => setStage({ kind: "starting", attempt: attempt + 1 })}>
                Retry
              </Button>
            </div>
          </section>
        </main>
      );
    default:
      return <Intro progress={progressOf(stage)} animate={animate} />;
  }
}

function progressOf(stage: Stage): IntroProgress {
  if (stage.kind === "starting") return { label: STARTING, done: 0, total: 1 + TOOL_ROWS.length };
  if (stage.kind === "checking") {
    const tools = stage.answer?.tools ?? [];
    const row = tools[stage.shown];
    if (row) return { label: checkingLabel(row.tool), done: 1 + stage.shown, total: 1 + tools.length };
  }
  return { label: checkingLabel("git"), done: 1, total: 1 + TOOL_ROWS.length };
}

/** The logo, turning while animations are on, above a bar that names the startup step under way. */
function Intro({ progress, animate }: { progress: IntroProgress; animate: boolean }) {
  return (
    <main className="startup">
      <div className="intro">
        <RadarMark className="boot-logo" spin={animate} />
        <progress className="boot-bar" value={progress.done} max={progress.total} />
        <p className="boot-label">{progress.label}</p>
      </div>
    </main>
  );
}
