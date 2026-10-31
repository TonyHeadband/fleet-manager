import { useRef, type ReactNode } from "react";
import { Carousel } from "../ui/Carousel";
import { Tabs } from "../ui/Tabs";
import { navigate } from "./routes";

/** egui's `tab_bar` over a view's body, as Fleet and Settings draw it; `body` draws each tab, so the view owns no
 * tab's state. The picked tab lives in the address (`href`). With `groups`, a row of groups comes first and the tab row
 * shows the picked group's tabs; a group opens on the tab last picked in it. */
export function TabbedView<Id extends string, Group extends string = never>(props: {
  className: string;
  /** The view's name, which names its tab lists ("Settings tabs"). */
  name: string;
  tabs: readonly { id: Id; label: string; group?: Group }[];
  groups?: readonly { id: Group; label: string }[];
  selected: Id;
  href: (tab: Id) => string;
  body: (tab: Id) => ReactNode;
}) {
  const { className, name, tabs, groups, selected, href, body } = props;
  const group = tabs.find((t) => t.id === selected)?.group;
  const last = useRef(new Map<Group, Id>());
  if (group !== undefined) last.current.set(group, selected);
  const shown = groups ? tabs.filter((t) => t.group === group) : tabs;
  return (
    <div className={`tabbed ${className}`}>
      {groups && group !== undefined && (
        <Tabs
          label={`${name} groups`}
          className="tab-groups"
          tabs={groups}
          selected={group}
          onSelect={(g) => navigate(href(last.current.get(g) ?? tabs.find((t) => t.group === g)!.id))}
        />
      )}
      <Tabs label={`${name} tabs`} tabs={shown} selected={selected} onSelect={(t) => navigate(href(t))} />
      {/* The track runs through every tab, across groups, so a step to a neighbouring group's tab is one pane. */}
      <div className="tabbed-body">
        <Carousel ids={tabs.map((t) => t.id)} current={selected} keep="sliding" render={body} />
      </div>
    </div>
  );
}
