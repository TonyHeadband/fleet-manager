import { useEffect, useState } from "react";
import type { FleetClient } from "../api/client";
import type { Native } from "../native/native";
import type { RestartApi } from "../settings/restart";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";

interface Options {
  client: FleetClient;
  native: Native | null;
  /** This window started the daemon: it is this release's, nothing to offer. */
  weStarted: boolean;
  /** The first agent snapshot has loaded: the offer never covers an empty board. */
  loaded: boolean;
  /** Restart runs the restart Settings' Restart daemon runs, which asks first while agents run. */
  restart: Pick<RestartApi, "ask" | "restarting" | "confirming">;
}

/**
 * egui's restart offer: a daemon this window did not start, from another release, is offered a restart, once. `open`
 * while it shows, so the update notice waits for it as egui's waits for any other dialog.
 */
export function useVersionOffer({ client, native, weStarted, loaded, restart }: Options) {
  // undefined: not asked or the same version; null: a daemon with no version route.
  const [found, setFound] = useState<string | null | undefined>(undefined);
  // Restart was clicked: the offer stays up with Settings' "Restarting the daemon…" until the restart settles, or
  // until its confirmation is cancelled.
  const [sent, setSent] = useState(false);
  const ask = !!native && !weStarted && loaded;
  const running = restart.restarting !== null;

  useEffect(() => {
    if (!ask || !native) return;
    let live = true;
    Promise.all([client.version(), native.version()]).then(
      ([info, mine]) => live && info?.version !== mine && setFound(info?.version ?? null),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [ask, client, native]);

  if (sent && !running && !restart.confirming) {
    setSent(false);
    setFound(undefined);
  }

  const open = found !== undefined && !!native;
  const from = found === null ? "an older version" : `v${found}`;
  const dialog = open && (
    // Answered by its buttons only, as egui's.
    <Modal label="Restart the background service" onClose={null}>
      <p className="dialog-body">The background service is from {from} — restart it?</p>
      <div className="form-actions">
        <Button
          variant="primary"
          disabled={sent}
          onClick={() => {
            setSent(true);
            restart.ask({ kind: "restart" });
          }}
        >
          Restart
        </Button>
        <Button disabled={sent} onClick={() => setFound(undefined)}>
          Keep using it
        </Button>
        {running && <span className="note">Restarting the daemon…</span>}
      </div>
    </Modal>
  );
  return { open, dialog };
}
