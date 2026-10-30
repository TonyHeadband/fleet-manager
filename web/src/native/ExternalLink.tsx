import type { Native } from "./native";

/** A link to a page outside Fleet Manager: the app opens it in the browser, a browser in a new tab. */
export function ExternalLink({ href, native }: { href: string; native: Native | null }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => {
        if (!native) return;
        e.preventDefault();
        void native.openUrl(href);
      }}
    >
      {href}
    </a>
  );
}
