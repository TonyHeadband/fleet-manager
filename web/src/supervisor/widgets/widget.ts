import type { JSX } from "react";
import type { EventSourceFactory } from "../../api/events";
import type { Agent } from "../../api/generated/Agent";
import type { FleetClient } from "../../api/client";
import type { Task } from "../../api/generated/Task";
import type { ArchiveTarget } from "../../tasks/ArchiveDialog";
import type { RoomsClient } from "./RoomChat";

export interface WidgetProps {
  persona: string;
  agents: Agent[];
  tasks: Task[];
  /** False until the first snapshot arrives, so a count is not shown as 0 before it is known. */
  loaded: boolean;
  /** The daemon, for a widget that reads more than the board has (the rooms). */
  baseUrl: string;
  client: RoomsClient & Pick<FleetClient, "storage">;
  /** Opens the app's archive dialog for a task. */
  onArchive: (target: ArchiveTarget) => void;
  factory?: EventSourceFactory;
}

/** What the Supervisor page's grid, and later the board, needs to place and draw a widget. */
export interface WidgetDef {
  id: string;
  title: string;
  /** Smallest size in grid cells. */
  minW: number;
  minH: number;
  render(p: WidgetProps): JSX.Element;
}
