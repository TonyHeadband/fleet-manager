import { useEffect, useRef, useState, type ReactNode } from "react";
import type { DiffReport } from "../api/generated/DiffReport";
import { ChatPane, type ChatClient } from "../chat/ChatPane";
import type { ChatSpeakers } from "../chat/speakers";
import { DiffView } from "../diff/DiffView";
import type { EventSourceFactory } from "../api/events";
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
  /** Loads the chat's earlier history and answers the agent's prompts. */
  client: ChatClient;
  /** Drawn only when the pane stands alone; inside the detail panel, the panel's top row carries it. */
  title?: string;
  factory?: EventSourceFactory;
  /** Who the chat shows the owner and the agent as. */
  speakers: ChatSpeakers;
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

export type Diff =
  { state: "none" } | { state: "loading" } | { state: "loaded"; report: DiffReport } | { state: "failed"; message: string };

/**
 * The detail panel's tabs: the agent's chat, where its prompts are answered; the diff, loaded once and kept until
 * Refresh or another agent. The live TUI stays "Open in terminal" or `fleet-attach`.
 */
export function PreviewPane(props: Props) {
  // Another agent starts afresh from its first frame: a reset after the render would paint one frame of the previous
  // agent's tab (its Diff or Breakdown) first.
  return <AgentPane key={props.agentId} {...props} />;
}

function AgentPane({ baseUrl, agentId, client, title, factory, speakers, loadDiff, extraTabs = [], tab, onTab, onShown, onClose }: Props) {
  // A newly opened agent starts on the tab that waits on the owner if one does (Lite's `opened_fleet_panel`); a tab
  // turning `first` later does not pull the owner away.
  const [ownTab, setOwnTab] = useState(() => extraTabs.find((t) => t.first)?.id ?? "chat");
  const [diff, setDiff] = useState<Diff>({ state: "none" });
  // Only the latest diff request's answer is shown: pressing Refresh again makes any answer still out a stale one.
  const asked = useRef(0);
  const current = tab ?? ownTab;

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

  const own = [{ id: "chat", label: "Chat" }, ...(loadDiff ? [{ id: "diff", label: "Diff" }] : [])];
  const tabs = [...extraTabs.filter((t) => t.first), ...own, ...extraTabs.filter((t) => !t.first)];
  // The Breakdown tab of an orchestrator left for another panel falls back to the chat.
  const shownTab = tabs.some((t) => t.id === current) ? current : "chat";
  useEffect(() => onShown?.(shownTab), [shownTab, onShown]);
  const pane = (id: string) => {
    const extra = extraTabs.find((t) => t.id === id);
    if (extra) return extra.node;
    if (id === "diff") return <DiffView diff={diff} />;
    return <ChatPane baseUrl={baseUrl} agentId={agentId} client={client} factory={factory} speakers={speakers} />;
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
