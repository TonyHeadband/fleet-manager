import { useState, type ReactNode } from "react";
import { ErrorLine } from "../ui/ErrorLine";
import { PlusMinus } from "../ui/PlusMinus";
import { Folding } from "./Folding";
import { Markdown } from "./Markdown";

interface Props {
  /** What the asker wants, in the accent mono caps; it also names the group. */
  caption: string;
  /** What is asked, on one line: an answered ask folds to it. */
  summary: string;
  /** The answer's label once answered ("" when it is not known), or null while the ask is open. */
  answered: string | null;
  error?: string | null;
  /** What is asked, and while it is open, the way to answer it. */
  children: ReactNode;
}

const oneLine = (text: string) => text.replace(/\s+/g, " ").trim();

/**
 * An ask in a chat, in the accent outline while open, the text colour once answered. Open, it is whole: its caption,
 * what is asked, the way to answer. Answered, it folds to one line (caption, the ask, the answer, both cut to the
 * width), and opens again from there.
 */
export function AskFrame({ caption, summary, answered, error, children }: Props) {
  const [open, setOpen] = useState(false);
  if (answered === null)
    return (
      <div role="group" aria-label={caption} className="chat-ask">
        <p className="chat-ask-caption">{caption}</p>
        {children}
        {error && <ErrorLine text={error} />}
      </div>
    );
  const head = (
    <button type="button" className="chat-ask-line" aria-expanded={open} onClick={() => setOpen(!open)}>
      <PlusMinus open={open} />
      <span className="chat-ask-caption">{caption}</span>
      {!open && <span className="chat-ask-summary">{oneLine(summary)}</span>}
      {!open && <span className="chat-ask-caption chat-ask-said">{answered ? `Answered · ${oneLine(answered)}` : "Answered"}</span>}
    </button>
  );
  return (
    <div role="group" aria-label={caption} className="chat-ask" data-answered="" data-compact={open ? undefined : ""}>
      {head}
      {open && (
        <>
          {children}
          <Answer text={answered} />
        </>
      )}
      {error && <ErrorLine text={error} />}
    </div>
  );
}

/** The answer under its ask, folded past eight lines like any message: a one-line answer can still be a long one. */
function Answer({ text }: { text: string }) {
  if (!text) return <p className="chat-ask-answered">Answered</p>;
  if (!text.includes("\n"))
    return (
      <Folding>
        <p className="chat-ask-answered">Answered · {text}</p>
      </Folding>
    );
  return (
    <>
      <p className="chat-ask-answered">Answered</p>
      <Folding>
        <Markdown text={text} />
      </Folding>
    </>
  );
}
