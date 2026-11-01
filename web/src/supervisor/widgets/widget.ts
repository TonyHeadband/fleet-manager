import type { JSX } from "react";
import type { EventSourceFactory } from "../../api/events";
import type { Agent } from "../../api/generated/Agent";
import type { Task } from "../../api/generated/Task";
import type { RoomsClient } from "./RoomChat";

export interface WidgetProps {
  persona: string;
  agents: Agent[];
  tasks: Task[];
  /** False until the first snapshot arrives, so a count is not shown as 0 before it is known. */
  loaded: boolean;
  /** The daemon, for a widget that reads more than the board has (the rooms). */
  baseUrl: string;
  client: RoomsClient;
  factory?: EventSourceFactory;
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
