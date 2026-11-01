import type { JSX } from "react";
import type { Agent } from "../../api/generated/Agent";
import type { Task } from "../../api/generated/Task";

export interface WidgetProps {
  persona: string;
  agents: Agent[];
  tasks: Task[];
}

/** What the Supervisor page's grid, and later the board, needs to place and draw a widget. */
export interface WidgetDef {
  id: string;
  title: string;
  /** Where the numbers come from, shown in the widget's footer. */
  source: string;
  /** Smallest size in grid cells. */
  minW: number;
  minH: number;
  render(p: WidgetProps): JSX.Element;
}
