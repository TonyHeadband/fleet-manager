import { useEffect, useState } from "react";
import type { FleetClient } from "../api/client";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";

interface Props {
  client: FleetClient;
  native: Native | null;
  /** This window started the daemon: it is this release's, nothing to offer. */
  weStarted: boolean;
  /** The first agent snapshot has loaded: the offer never covers an empty board. */
  loaded: boolean;
  /** After Restart: the page starts over against the new daemon. */
  onRestarted: () => void;
}

/** egui's restart offer: a daemon this window did not start, from another release, is offered a restart, once. */
export function VersionOffer({ client, native, weStarted, loaded, onRestarted }: Props) {
  // undefined: not asked or the same version; null: a daemon with no version route.
  const [found, setFound] = useState<string | null | undefined>(undefined);
  const ask = !!native && !weStarted && loaded;

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

  if (found === undefined || !native) return null;
  const from = found === null ? "an older version" : `v${found}`;
  return (
    <div className="dialog-backdrop">
      <section className="dialog" role="dialog" aria-modal="true" aria-label="Restart the background service">
        <p className="dialog-body">The background service is from {from} — restart it?</p>
        <div className="form-actions">
          <Button
            variant="primary"
            onClick={() => {
              setFound(undefined);
              native.restartDaemon().then(onRestarted, () => {});
            }}
          >
            Restart
          </Button>
          <Button onClick={() => setFound(undefined)}>Keep using it</Button>
        </div>
      </section>
    </div>
  );
}
