import MarkdownIt from "markdown-it";
import { useMemo } from "react";

// Raw HTML off: a tag in Claude's text is escaped and shows as text. markdown-it also refuses `javascript:` and other
// unsafe link targets. Images are off too, so a reply never loads anything from the network.
const md = new MarkdownIt({ html: false, linkify: true }).disable("image");

const renderLinkOpen = md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
// Links open outside the app: a new tab in a browser; in the app, `installAppNative` hands these to the opener.
md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  tokens[idx]!.attrSet("target", "_blank");
  tokens[idx]!.attrSet("rel", "noreferrer");
  return renderLinkOpen(tokens, idx, options, env, self);
};

/** Claude's rich text: paragraphs, emphasis, inline code, code blocks, lists and links. */
export function Markdown({ text }: { text: string }) {
  const html = useMemo(() => md.render(text), [text]);
  // markdown-it's own markup over escaped text (html: false): nothing from the text reaches the DOM as markup.
  return <div className="markdown" dangerouslySetInnerHTML={{ __html: html }} />;
}
