import { useState } from "react";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";
import { useAppVersion, useUpdateCheck } from "./update";

/**
 * egui's new-version notice: a newer release than this app, unless skipped. It waits while another dialog is up
 * (`blocked`), the first run's wizard included.
 */
export function UpdateNotice({ native, blocked }: { native: Native | null; blocked: boolean }) {
  const check = useUpdateCheck(native);
  const current = useAppVersion(native);
  // The release the notice was closed on, so it stays away until a later one; a skipped release is Settings' alone.
  const [closed, setClosed] = useState<string | null>(null);
  const [skip, setSkip] = useState(false);
  const release = check.kind === "available" && !check.skipped && check.version !== closed ? check : null;

  if (!native || !release || blocked) return null;
  // Either way out keeps a ticked "Skip this version".
  const close = () => {
    if (skip) void native.skipVersion(release.version);
    setClosed(release.version);
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
