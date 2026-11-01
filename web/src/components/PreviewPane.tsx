import { useEffect, useRef, useState, type ReactNode } from "react";
import type { DiffReport } from "../api/generated/DiffReport";
import { DiffView } from "../diff/DiffView";
import { browserEventSource, subscribePane, type EventSourceFactory } from "../api/events";
import { Button } from "../ui/Button";
import { Carousel } from "../ui/Carousel";
import { Tabs } from "../ui/Tabs";

/** A tab the caller adds (the orchestrator's Breakdown): first when it needs a look, else after Diff. */
export interface ExtraTab {
  id: string;
  label: string;
  first: boolean;
  node: ReactNode;
}

interface Props {
  baseUrl: string;
  agentId: string;
  /** Drawn only when the pane stands alone; inside the detail panel, the panel's top row carries it. */
  title?: string;
  factory?: EventSourceFactory;
  /** Loads the agent's changes; without it there is no Diff tab. */
  loadDiff?: () => Promise<DiffReport>;
  extraTabs?: ExtraTab[];
  /** The tab shown, when the caller picks it (an orchestrator waiting on its breakdown opens on Breakdown). */
  tab?: string;
  onTab?: (id: string) => void;
  /** The tab now shown, as it changes (the panel goes full for the Diff). */
  onShown?: (id: string) => void;
  onClose?: () => void;
}

/** Lite's `MAX_LINE_CHARS`: a longer line is cut, so one runaway line cannot stall the pane. */
const MAX_LINE_CHARS = 2000;

export type Diff =
  { state: "none" } | { state: "loading" } | { state: "loaded"; report: DiffReport } | { state: "failed"; message: string };

/** The daemon sends the whole screen, empty rows under short output included: kept, they would push the text out of
 * the bottom-stuck view. */
export const screenText = (text: string) =>
  text
    .trimEnd()
    .split("\n")
    .map((line) => [...line].slice(0, MAX_LINE_CHARS).join(""))
    .join("\n");

/**
 * Lite's panel tabs: the terminal, read-only, stuck to its bottom; the diff, loaded once and kept until Refresh or
 * another agent. Read-only by design: use "Open in terminal" or `fleet-attach` for control.
 */
export function PreviewPane(props: Props) {
  // Another agent starts afresh from its first frame: a reset after the render would paint one frame of the previous
  // agent's tab (its Diff or Breakdown) first.
  return <AgentPane key={props.agentId} {...props} />;
}

function AgentPane({ baseUrl, agentId, title, factory = browserEventSource, loadDiff, extraTabs = [], tab, onTab, onShown, onClose }: Props) {
  // A newly opened agent starts on the tab that waits on the owner if one does (Lite's `opened_fleet_panel`); a tab
  // turning `first` later does not pull the owner away.
  const [ownTab, setOwnTab] = useState(() => extraTabs.find((t) => t.first)?.id ?? "terminal");
  const [diff, setDiff] = useState<Diff>({ state: "none" });
  const [text, setText] = useState("");
  const [closed, setClosed] = useState(false);
  const pre = useRef<HTMLPreElement>(null);
  // Only the latest diff request's answer is shown: pressing Refresh again makes any answer still out a stale one.
  const asked = useRef(0);
  const current = tab ?? ownTab;

  useEffect(() => {
    setText("");
    setClosed(false);
    return subscribePane(baseUrl, agentId, setText, () => setClosed(true), factory);
  }, [baseUrl, agentId, factory]);

  useEffect(() => {
    if (pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [text, current]);

  const load = () => {
    if (!loadDiff) return;
    const n = ++asked.current;
    setDiff({ state: "loading" });
    loadDiff().then(
      (report) => n === asked.current && setDiff({ state: "loaded", report }),
      (e: unknown) => n === asked.current && setDiff({ state: "failed", message: e instanceof Error ? e.message : String(e) }),
    );
  };

  const select = (id: string) => {
    setOwnTab(id);
    onTab?.(id);
    if (id === "diff" && diff.state === "none") load();
  };

  const own = [{ id: "terminal", label: "Terminal" }, ...(loadDiff ? [{ id: "diff", label: "Diff" }] : [])];
  const tabs = [...extraTabs.filter((t) => t.first), ...own, ...extraTabs.filter((t) => !t.first)];
  // The Breakdown tab of an orchestrator left for another panel falls back to the terminal.
  const shownTab = tabs.some((t) => t.id === current) ? current : "terminal";
  useEffect(() => onShown?.(shownTab), [shownTab, onShown]);
  const pane = (id: string) => {
    const extra = extraTabs.find((t) => t.id === id);
    if (extra) return extra.node;
    if (id === "diff") return <DiffView diff={diff} />;
    return (
      <>
        {closed && <p className="note ended">Session ended</p>}
        <pre ref={pre} className="pane" aria-label="Terminal preview">
          {screenText(text)}
        </pre>
      </>
    );
  };

  return (
    <div className="preview">
      {title !== undefined && (
        <header>
          <h2>{title}</h2>
          {onClose && <Button onClick={onClose}>Close</Button>}
        </header>
      )}
      {tabs.length > 1 && (
        <Tabs tabs={tabs.map(({ id, label }) => ({ id, label }))} selected={shownTab} onSelect={select}>
          {shownTab === "diff" && (
            <Button className="refresh" onClick={load}>
              Refresh
            </Button>
          )}
        </Tabs>
      )}
      <div className="pane-frame">
        <Carousel ids={tabs.map((t) => t.id)} current={shownTab} keep="visited" render={pane} />
      </div>
    </div>
  );
}
