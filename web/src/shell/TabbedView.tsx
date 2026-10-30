import type { ReactNode } from "react";
import { Tabs } from "../ui/Tabs";
import { navigate } from "./routes";

/** egui's `tab_bar` over a view's body, as Management and Settings draw it; `body` draws each tab, so the view owns
 * no tab's state. The picked tab lives in the address (`href`). */
export function TabbedView<Id extends string>(props: {
  className: string;
  tabs: readonly { id: Id; label: string }[];
  selected: Id;
  href: (tab: Id) => string;
  body: (tab: Id) => ReactNode;
}) {
  const { className, tabs, selected, href, body } = props;
  return (
    <div className={`tabbed ${className}`}>
      <Tabs tabs={tabs} selected={selected} onSelect={(t) => navigate(href(t))} />
      <div className="tabbed-body">{body(selected)}</div>
    </div>
  );
}
