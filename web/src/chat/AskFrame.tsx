import type { ReactNode } from "react";
import { ErrorLine } from "../ui/ErrorLine";

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
      {answered !== null && <p className="chat-ask-answered">{answered ? `Answered · ${answered}` : "Answered"}</p>}
      {error && <ErrorLine text={error} />}
    </div>
  );
}
