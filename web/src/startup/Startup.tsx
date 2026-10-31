import { useEffect, useRef, useState, type ReactNode } from "react";
import type { FleetClient } from "../api/client";
import type { ToolList } from "../api/generated/ToolList";
import { missingRequired, toolName } from "../api/tools";
import { takeDaemonToken, type Native } from "../native/native";
import { RadarMark } from "../shell/RadarMark";
import { bootBarWidth, BootMark } from "./BootMark";
import { BOOT_MS } from "./bootTimeline";
import { Button } from "../ui/Button";
import { checkingLabel, RETRY_MS, ROW_MS, STARTING, TOOL_ROWS, wizardHeading, type IntroProgress, type WizardSteps } from "./boot";
import { FirstRun } from "./FirstRun";
import { Requirements } from "./Requirements";
import { useToolCheck } from "./toolCheck";
import "./startup.css";

/**
 * What startup learned, for the board: the daemon's client, whether this window started it, whether it is on this
 * machine (native actions are offered only then), and the tools.
 */
export interface Started {
  client: FleetClient;
  weStarted: boolean;
  local: boolean;
  tools: ToolList | null;
  /** The wizard steps shown before the board, which the persona setup counts after. */
  steps: WizardSteps;
  /** Required tools the person continued without. */
  stillMissing: string[];
}

type Up = Pick<Started, "client" | "weStarted" | "local">;

type Stage =
  | { kind: "starting"; attempt: number }
  | { kind: "firstRun"; attempt: number; defaults: { root: string; personas: string }; error: string | null }
  | { kind: "failed"; attempt: number; message: string }
  /** `answer` is undefined until the tools check answers, null when it failed (with `error`). */
  | { kind: "checking"; up: Up; answer?: ToolList | null; error?: string; shown: number }
  | { kind: "requirements"; up: Up; list: ToolList; note: string | null }
  | { kind: "running"; started: Started };

const NO_TOOLS: ToolList = { tools: [], checked_at: 0n, path: null };

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
 * The window until the board, as egui's: the folders step on a first run, the intro while the daemon comes up and
 * answers about the tools (or the failure screen with Retry), and Requirements on a first run or when a required
 * tool is missing. In a browser there is no daemon to start and the config exists, so the intro waits until the
 * daemon answers and the folders step never shows.
 */
export function Startup({ connect, browserUrl, native, animate, children }: Props) {
  const [stage, setStage] = useState<Stage>({ kind: "starting", attempt: 0 });
  const [stateDir, setStateDir] = useState("");
  // The folders step was shown this run: Requirements follows it even with every tool found, and both are counted.
  const folders = useRef(false);
  const launch = useRef<{ attempt: number; stage: Promise<Stage> } | null>(null);
  const attempt = stage.kind === "starting" ? stage.attempt : -1;
  // The app's boot mark assembles once per launch, timed from the first render; until it has, every stage waits on
  // the intro, which keeps the progress it last showed. The browser keeps its plain intro and never waits.
  const startedAt = useRef<number | null>(null);
  if (native && startedAt.current === null) startedAt.current = performance.now();
  const [assembled, setAssembled] = useState(!(native && animate));
  const lastProgress = useRef<IntroProgress | null>(null);
  useEffect(() => {
    if (assembled) return;
    const timer = setTimeout(() => setAssembled(true), BOOT_MS);
    return () => clearTimeout(timer);
  }, [assembled]);

  useEffect(() => {
    if (attempt < 0) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (native) {
      // One launch per attempt, however often the effect runs (React runs it twice in development).
      if (launch.current?.attempt !== attempt) launch.current = { attempt, stage: launchNative(native, attempt, connect, folders) };
      void launch.current.stage.then((next) => live && setStage(next));
    } else {
      const client = connect(browserUrl);
      const ask = () =>
        client.uiSchema().then(
          () => live && setStage({ kind: "checking", up: { client, weStarted: false, local: false }, shown: 0 }),
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

  // The terminals this app opens find what the daemon found, when the daemon is on this machine.
  const adopt = (up: Up, list: ToolList) => {
    if (native && up.local && list.path) void native.adoptPath();
  };

  const checkingUp = stage.kind === "checking" ? stage.up : null;
  useEffect(() => {
    if (!checkingUp) return;
    let live = true;
    // An unanswered check never blocks the board: egui moves on with no tool list.
    checkingUp.client.tools().then(
      (answer) => {
        adopt(checkingUp, answer);
        if (live) setStage((s) => (s.kind === "checking" ? { ...s, answer } : s));
      },
      (e: unknown) =>
        live && setStage((s) => (s.kind === "checking" ? { ...s, answer: null, error: e instanceof Error ? e.message : String(e) } : s)),
    );
    return () => {
      live = false;
    };
    // `adopt` reads only props that never change for a mounted window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checkingUp]);

  const answer = stage.kind === "checking" ? stage.answer : undefined;
  const stepping = animate && answer != null && answer.tools.length > 0;
  useEffect(() => {
    if (answer === undefined) return;
    // egui's `after_check`: Requirements on a first run or when a required tool is missing, else the board.
    const next = (s: Stage): Stage => {
      if (s.kind !== "checking") return s;
      const list = s.answer ?? NO_TOOLS;
      const note = s.error === undefined ? null : `Unable to check the tools: ${s.error}`;
      if (folders.current || missingRequired(list)) return { kind: "requirements", up: s.up, list, note };
      return {
        kind: "running",
        started: { ...s.up, tools: s.answer ?? null, steps: { folders: folders.current, requirements: false }, stillMissing: [] },
      };
    };
    if (!stepping) {
      setStage(next);
      return;
    }
    const timer = setInterval(
      () =>
        setStage((s) => {
          if (s.kind !== "checking" || !s.answer) return s;
          return s.shown + 1 >= s.answer.tools.length ? next(s) : { ...s, shown: s.shown + 1 };
        }),
      ROW_MS,
    );
    return () => clearInterval(timer);
  }, [answer, stepping]);

  const steps = { folders: folders.current, requirements: true };
  const intro = (progress: IntroProgress) => <Intro progress={progress} animate={animate} startedAt={startedAt.current} />;
  const onIntro = stage.kind === "starting" || stage.kind === "checking";
  if (onIntro) lastProgress.current = progressOf(stage);
  if (!assembled && !onIntro) {
    const last = lastProgress.current ?? progressOf(stage);
    // Past the checks (the board or Requirements is next), the bar shows them done rather than stalling short of full.
    const done = stage.kind === "running" || stage.kind === "requirements";
    return intro(done ? { ...last, done: last.total } : last);
  }
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
              <Button variant="primary" onClick={() => setStage({ kind: "starting", attempt: stage.attempt + 1 })}>
                Retry
              </Button>
            </div>
          </section>
        </main>
      );
    case "firstRun":
      return (
        <main className="startup">
          <FirstRun
            native={native!}
            heading={wizardHeading(steps, "folders", "Where Fleet Manager lives")}
            defaults={stage.defaults}
            error={stage.error}
            onNext={(root, personas) =>
              void native!.writeInitialConfig(root, personas).then(
                () => setStage({ kind: "starting", attempt: stage.attempt + 1 }),
                (e: unknown) => setStage({ ...stage, error: String(e) }),
              )
            }
          />
        </main>
      );
    case "requirements":
      return (
        <main className="startup">
          <FirstRequirements
            client={stage.up.client}
            native={native}
            local={stage.up.local}
            heading={wizardHeading(steps, "requirements", "What Fleet Manager needs")}
            first={{ list: stage.list, note: stage.note }}
            onContinue={(list) =>
              setStage({
                kind: "running",
                started: {
                  ...stage.up,
                  tools: list,
                  steps,
                  stillMissing: list.tools.filter((t) => t.required && t.error).map((t) => toolName(t.tool)),
                },
              })
            }
          />
        </main>
      );
    default:
      return intro(progressOf(stage));
  }
}

/**
 * The app's start: the folders step on a first run, else the daemon found or started. As egui, the first run is
 * decided at launch; after the folders step, and on Retry, the start goes ahead.
 */
async function launchNative(
  native: Native,
  attempt: number,
  connect: (url: string) => FleetClient,
  folders: { current: boolean },
): Promise<Stage> {
  if (attempt === 0 && !(await native.hasConfig().catch(() => true))) {
    const defaults = await native.firstRunDefaults().catch(() => ({ root: "~/fleet-tasks", personas: "" }));
    folders.current = true;
    return { kind: "firstRun", attempt, defaults, error: null };
  }
  try {
    const r = await native.startDaemon();
    const client = connect(r.url);
    await takeDaemonToken(native, client);
    return { kind: "checking", up: { client, weStarted: r.weStarted, local: r.local }, shown: 0 };
  } catch (e) {
    return { kind: "failed", attempt, message: String(e) };
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

/**
 * The mark above a bar that names the startup step under way. In the app (`startedAt` set) the mark assembles itself
 * and the bar spans its ring; in the browser it is the finished mark, its ring turning while animations are on.
 */
function Intro({ progress, animate, startedAt }: { progress: IntroProgress; animate: boolean; startedAt: number | null }) {
  return (
    <main className="startup">
      <div className="intro">
        {startedAt === null ? <RadarMark className="boot-logo" spin={animate} /> : <BootMark startedAt={startedAt} animate={animate} />}
        <progress
          className={startedAt === null ? "boot-bar" : "boot-bar app"}
          style={startedAt === null ? undefined : { width: bootBarWidth() }}
          value={progress.done}
          max={progress.total}
        />
        <p className="boot-label">{progress.label}</p>
      </div>
    </main>
  );
}

/** The first run's Requirements, seeded with the answer the intro waited for. */
function FirstRequirements(props: {
  client: FleetClient;
  native: Native | null;
  local: boolean;
  heading: string;
  first: { list: ToolList; note: string | null };
  onContinue: (list: ToolList) => void;
}) {
  const { client, native, local, heading, first, onContinue } = props;
  const check = useToolCheck(client, native, { canInstall: native !== null && local, local, first });
  return <Requirements native={native} check={check} heading={heading} onContinue={onContinue} />;
}
