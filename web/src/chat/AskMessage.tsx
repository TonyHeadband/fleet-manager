import { useState } from "react";
import { FleetError } from "../api/client";
import type { AskAnswer } from "../api/generated/AskAnswer";
import type { AskKind } from "../api/generated/AskKind";
import type { AskQuestion } from "../api/generated/AskQuestion";
import type { ChatAsk } from "../api/generated/ChatAsk";
import { screenText } from "../ui/screenText";
import { Button } from "../ui/Button";
import { AskFrame } from "./AskFrame";
import type { ChatClient } from "./ChatPane";

interface Props {
  agentId: string;
  ask: ChatAsk;
  client: ChatClient;
  /** The answer went through: the label of the choice, or "" when the daemon says it was already answered. */
  onAnswered: (label: string) => void;
}

const CAPTION = { permission: "Wants to run", question: "Asks", screen: "Waiting on its screen" } as const;

/** A tool's input as the owner reads it: Bash's command whole, anything else as its JSON. */
function inputText(input: unknown): string {
  if (input && typeof input === "object" && "command" in input && typeof input.command === "string") return input.command;
  return JSON.stringify(input, null, 2);
}

/** What was asked, for an answered ask's one line. */
function summaryOf(kind: AskKind): string {
  if (kind.type === "permission") return `${kind.tool} ${inputText(kind.input)}`;
  if (kind.type === "question") return kind.questions.map((q) => q.question).join(" · ");
  return "";
}

/**
 * A prompt the agent waits on, in the accent outline, as the Claude app shows one: a permission's call and its
 * options, Claude's questions with theirs, or, for a prompt nothing describes, the agent's screen. Pressing an option
 * answers it; it then reads as answered, the buttons gone.
 */
export function AskMessage({ agentId, ask, client, onAnswered }: Props) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const kind = ask.kind;

  const send = async (answer: AskAnswer, label: string) => {
    setSending(true);
    setError(null);
    try {
      await client.answerAsk(agentId, ask.id, answer);
      onAnswered(label);
    } catch (e) {
      // Answered meanwhile (in the terminal, or by another window): nothing is left to answer.
      if (e instanceof FleetError && e.status === 409) onAnswered("");
      else setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  let controls;
  if (kind.type === "permission") {
    controls = (
      <div className="chat-ask-options" data-row>
        {kind.options.map((o, i) => (
          <Button key={o.id} variant={i === 0 ? "primary" : "default"} disabled={sending} onClick={() => send({ choice: o.id, text: null }, o.label)}>
            {o.label}
          </Button>
        ))}
      </div>
    );
  } else if (kind.type === "question") {
    controls = <Questions questions={kind.questions} sending={sending} onSend={send} />;
  } else {
    controls = <p className="chat-ask-note">Answer it in the reply box.</p>;
  }

  return (
    <AskFrame caption={CAPTION[kind.type]} summary={summaryOf(kind)} answered={ask.answered} error={error}>
      {kind.type === "permission" && (
        <div className="chat-ask-call">
          <span className="chat-call-tool">{kind.tool}</span>
          <pre>{inputText(kind.input)}</pre>
        </div>
      )}
      {kind.type === "screen" && <pre className="chat-ask-screen">{screenText(kind.text)}</pre>}
      {ask.answered === null && controls}
    </AskFrame>
  );
}

/**
 * Claude's questions, each with its options. A lone single-choice question answers on a press; otherwise the options
 * toggle and Send answers them all. One free-text answer stands for the one question no option answers; it is sent
 * only by its own submit (Send or Enter), never by an option's press.
 */
function Questions({ questions, sending, onSend }: { questions: AskQuestion[]; sending: boolean; onSend: (a: AskAnswer, label: string) => void }) {
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [text, setText] = useState("");
  const direct = questions.length === 1 && !questions[0]!.multi_select;
  const ready = !sending && (picked.size > 0 || text.trim() !== "");

  const label = (ids: ReadonlySet<string>, free: string) => {
    const parts = questions.map((q) =>
      q.options
        .filter((o) => ids.has(o.id))
        .map((o) => o.label)
        .join(", "),
    );
    const blank = parts.findIndex((p) => p === "");
    if (blank >= 0 && free) parts[blank] = free;
    return parts.filter((p) => p !== "").join("; ");
  };
  const submit = (ids: ReadonlySet<string>, free: string) => onSend({ choice: [...ids].join(","), text: free || null }, label(ids, free));
  const press = (q: number, id: string) => {
    if (direct) return submit(new Set([id]), "");
    const next = new Set(picked);
    if (next.has(id)) next.delete(id);
    else {
      // A single-choice question keeps one pick.
      if (!questions[q]!.multi_select) for (const o of questions[q]!.options) next.delete(o.id);
      next.add(id);
    }
    setPicked(next);
  };

  return (
    <>
      {questions.map((q, i) => (
        <div key={i} className="chat-question">
          {q.header && <p className="chat-ask-caption">{q.header}</p>}
          <p>{q.question}</p>
          <div className="chat-ask-options" data-stack>
            {q.options.map((o) => (
              <Button key={o.id} pressed={direct ? undefined : picked.has(o.id)} disabled={sending} onClick={() => press(i, o.id)}>
                <span className="chat-option-label">{o.label}</span>
                {o.description && <span className="chat-option-description">{o.description}</span>}
              </Button>
            ))}
          </div>
        </div>
      ))}
      <form
        className="chat-ask-free"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) submit(picked, text.trim());
        }}
      >
        <input type="text" aria-label="Another answer" placeholder="Another answer…" value={text} onChange={(e) => setText(e.target.value)} disabled={sending} />
        <Button type="submit" variant="primary" disabled={!ready}>
          Send
        </Button>
      </form>
    </>
  );
}
