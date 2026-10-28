import { useEffect, useRef, useState } from "react";
import type { DiffReport } from "../api/generated/DiffReport";
import { browserEventSource, subscribePane, type EventSourceFactory } from "../api/events";
import { Button } from "../ui/Button";
import { Tabs } from "../ui/Tabs";

interface Props {
  baseUrl: string;
  agentId: string;
  title: string;
  factory?: EventSourceFactory;
  /** Loads the agent's changes; without it there is no Diff tab. */
  loadDiff?: () => Promise<DiffReport>;
  onClose?: () => void;
}

/** Read-only by design: it shows the pane and offers no way to type into it. Use "Open in terminal" or `fleet-attach` for control. */
export function PreviewPane({ baseUrl, agentId, title, factory = browserEventSource, loadDiff, onClose }: Props) {
  const [tab, setTab] = useState<"terminal" | "diff">("terminal");
  const [report, setReport] = useState<DiffReport | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [closed, setClosed] = useState(false);
  const pre = useRef<HTMLPreElement>(null);

  useEffect(() => {
    setText("");
    setClosed(false);
    return subscribePane(baseUrl, agentId, setText, () => setClosed(true), factory);
  }, [baseUrl, agentId, factory]);

  useEffect(() => {
    if (pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [text]);

  const refresh = () => {
    if (!loadDiff) return;
    setDiffError(null);
    loadDiff().then(setReport, (e: unknown) => setDiffError(e instanceof Error ? e.message : String(e)));
  };

  // A different agent, or leaving the tab, starts the diff view fresh.
  useEffect(() => {
    setReport(null);
    setDiffError(null);
    setTab("terminal");
  }, [agentId]);

  const openDiff = () => {
    setTab("diff");
    refresh();
  };

  return (
    <aside className="preview" aria-label="Preview">
      <header>
        <h2>{title}</h2>
        {onClose && <Button onClick={onClose}>Close</Button>}
      </header>
      {loadDiff && (
        <Tabs
          tabs={[{ id: "terminal", label: "Terminal" }, { id: "diff", label: "Diff" }] as const}
          selected={tab}
          onSelect={(id) => (id === "diff" ? openDiff() : setTab("terminal"))}
        >
          {tab === "diff" && <Button onClick={refresh}>Refresh</Button>}
        </Tabs>
      )}
      {tab === "terminal" ? (
        <>
          <pre ref={pre} aria-label="Terminal preview">
            {text}
          </pre>
          {closed && <p className="ended">Session ended</p>}
        </>
      ) : (
        <div className="diff" aria-label="Changes">
          {diffError && <p role="alert">{diffError}</p>}
          {report?.workspaces.map((w) => (
            <section key={w.repo}>
              <h3>{w.repo}</h3>
              {w.diff === "" ? <p className="empty">No changes</p> : <pre>{w.diff}</pre>}
              {w.truncated && <p className="empty">The diff was cut at the size limit.</p>}
            </section>
          ))}
        </div>
      )}
    </aside>
  );
}
