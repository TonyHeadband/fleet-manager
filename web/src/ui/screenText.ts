/** Lite's `MAX_LINE_CHARS`: a longer line is cut, so one runaway line cannot stall the view. */
const MAX_LINE_CHARS = 2000;

/** An agent's screen as text. The daemon sends the whole screen, empty rows under short output included: kept, they
 * would push the text out of a bottom-stuck view. */
export const screenText = (text: string) =>
  text
    .trimEnd()
    .split("\n")
    .map((line) => [...line].slice(0, MAX_LINE_CHARS).join(""))
    .join("\n");
