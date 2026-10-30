import { useEffect, useState } from "react";
import type { Native, Release } from "../native/native";
import { Button } from "../ui/Button";

/** egui's update check: at start and once a day. */
export const UPDATE_EVERY_MS = 24 * 3600 * 1000;

/**
 * egui's new-version notice: a newer release than this app, unless skipped. It waits while another dialog is up
 * (`blocked`), the first run's wizard included.
 */
export function UpdateNotice({ native, blocked }: { native: Native | null; blocked: boolean }) {
  const [release, setRelease] = useState<Release | null>(null);
  const [current, setCurrent] = useState("");
  const [skip, setSkip] = useState(false);

  useEffect(() => {
    if (!native) return;
    // A skipped release is Settings' to show, not the notice's.
    const check = () =>
      void native.checkUpdate().then(
        (c) => setRelease(c.kind === "available" && !c.skipped ? c : null),
        () => {},
      );
    native.version().then(setCurrent, () => {});
    check();
    const timer = setInterval(check, UPDATE_EVERY_MS);
    return () => clearInterval(timer);
  }, [native]);

  if (!native || !release || blocked) return null;
  // Either way out keeps a ticked "Skip this version".
  const close = () => {
    if (skip) void native.skipVersion(release.version);
    setRelease(null);
    setSkip(false);
  };
  return (
    <div className="dialog-backdrop">
      <section className="dialog update-notice" role="dialog" aria-modal="true">
        <h2>
          Fleet Manager v{release.version} is available (you have v{current})
        </h2>
        {release.notes.trim() && <p className="dialog-body notes">{release.notes.trim()}</p>}
        <label className="check">
          <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} />
          Skip this version
        </label>
        <div className="form-actions">
          <Button
            variant="primary"
            onClick={() => {
              void native.openUrl(release.page);
              close();
            }}
          >
            Download
          </Button>
          <Button onClick={close}>Later</Button>
        </div>
        <p className="note">Download opens the release page in your browser.</p>
      </section>
    </div>
  );
}
