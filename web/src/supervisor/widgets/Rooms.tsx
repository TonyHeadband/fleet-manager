import { useCallback, useEffect, useState } from "react";
import type { Post } from "../../api/generated/Post";
import type { Room } from "../../api/generated/Room";
import type { RoomKind } from "../../api/generated/RoomKind";
import { ErrorLine } from "../../ui/ErrorLine";
import { ListDetail } from "../../ui/ListDetail";
import { ListRow } from "../../ui/ListRow";
import { pad2 } from "../../ui/Readout";
import { RoomChat, message } from "./RoomChat";
import type { WidgetDef, WidgetProps } from "./widget";
import { Widget } from "./Widget";
import "./rooms.css";

const KIND: Record<RoomKind, string> = { operation: "Operation", product: "Product" };

function Rooms({ persona, agents, tasks, baseUrl, client, factory }: WidgetProps) {
  const [rooms, setRooms] = useState<Room[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // A task started is a room opened: ask again when the task list changes.
  const started = tasks.map((t) => t.id).join(" ");
  const reload = useCallback(() => {
    client.rooms(persona).then(setRooms, (e) => setError(message(e)));
  }, [client, persona]);
  useEffect(reload, [reload, started]);

  const list = rooms ?? [];
  const current = list.find((r) => r.id === picked) ?? list[0] ?? null;

  const markRead = (id: string) => setRooms((rs) => rs?.map((r) => (r.id === id ? { ...r, unread: 0 } : r)) ?? rs);
  const heard = (p: Post) => {
    if (rooms !== null && !rooms.some((r) => r.id === p.room)) return reload();
    // The open room is being read, and the owner's own posts are never unread.
    if (p.room === current?.id || p.sender === "owner") return;
    setRooms((rs) => rs?.map((r) => (r.id === p.room ? { ...r, unread: r.unread + 1 } : r)) ?? rs);
  };

  const rows = (
    <ul>
      {list.map((r) => (
        <li key={r.id}>
          <ListRow mark="current" selected={r.id === current?.id} onSelect={() => setPicked(r.id)} dimmed={r.archived_at != null}>
            <span className="list-row-name">{r.title}</span>
            <span className="rooms-kind mono-label">{KIND[r.kind]}</span>
            {r.unread > 0 && (
              <>
                <span className="rooms-mark" aria-hidden="true" />
                <span className="rooms-unread">
                  <span aria-hidden="true">{pad2(r.unread)}</span>
                  <span className="visually-hidden">{r.unread} unread</span>
                </span>
              </>
            )}
          </ListRow>
        </li>
      ))}
    </ul>
  );

  return (
    <Widget def={ROOMS}>
      {error && <ErrorLine text={error} />}
      <div className="rooms">
        <ListDetail
          width="task"
          listLabel="Rooms"
          detailLabel={current?.title ?? "Room"}
          list={rooms !== null && list.length === 0 ? <p className="rooms-empty">No rooms yet.</p> : rows}
          detail={
            current && (
              <RoomChat
                baseUrl={baseUrl}
                room={current}
                client={client}
                persona={persona}
                agents={agents}
                factory={factory}
                onPost={heard}
                onRead={() => markRead(current.id)}
              />
            )
          }
        />
      </div>
    </Widget>
  );
}

export const ROOMS: WidgetDef = {
  id: "rooms",
  title: "Rooms",
  source: "rooms",
  minW: 4,
  minH: 3,
  render: (p) => <Rooms {...p} />,
};
