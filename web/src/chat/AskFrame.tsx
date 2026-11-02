import type { ReactNode } from "react";
import { ErrorLine } from "../ui/ErrorLine";
import { Folding } from "./Folding";
import { Markdown } from "./Markdown";

interface Props {
  /** What the asker wants, in the accent mono caps; it also names the group. */
  caption: string;
  /** The answer's label once answered ("" when it is not known), or null while the ask is open. */
  answered: string | null;
  error?: string | null;
  /** What is asked, and while it is open, the way to answer it. */
  children: ReactNode;
}

/** An ask in a chat, in the accent outline while open, the text colour once answered: its caption, what is asked, then its answer once it has one. */
export function AskFrame({ caption, answered, error, children }: Props) {
  return (
    <div role="group" aria-label={caption} className="chat-ask" data-answered={answered !== null ? "" : undefined}>
      <p className="chat-ask-caption">{caption}</p>
      {children}
      {answered !== null && <Answer text={answered} />}
      {error && <ErrorLine text={error} />}
    </div>
  );
}

/** A one-line answer ("Approved") reads beside its caption; a longer one is its own folded block under it, so a long
 * answer is never a single wall-of-text caption line. */
function Answer({ text }: { text: string }) {
  if (!text.includes("\n")) return <p className="chat-ask-answered">{text ? `Answered · ${text}` : "Answered"}</p>;
  return (
    <>
      <p className="chat-ask-answered">Answered</p>
      <Folding>
        <Markdown text={text} />
      </Folding>
    </>
  );
}
