import { useState } from "react";
import type { Native, UpdateCheck } from "../native/native";
import { Button } from "../ui/Button";
import { Check } from "../ui/Check";
import { useAppVersion, useUpdateCheck } from "./update";

/**
 * egui's new-version notice: a newer release than this app, unless skipped. It waits while another dialog is up
 * (`blocked`), the first run's wizard included.
 */
export function UpdateNotice({ native, blocked }: { native: Native | null; blocked: boolean }) {
  const check = useUpdateCheck(native);
  const current = useAppVersion(native);
  // The answer the notice was closed on: the next check's answer brings it back, as egui's daily check does. A skipped
  // release is Settings' alone.
  const [closed, setClosed] = useState<UpdateCheck | null>(null);
  const [skip, setSkip] = useState(false);
  const release = check.kind === "available" && !check.skipped && check !== closed ? check : null;

  if (!native || !release || blocked) return null;
  // Either way out keeps a ticked "Skip this version".
  const close = () => {
    if (skip) void native.skipVersion(release.version);
    setClosed(release);
    setSkip(false);
  };
  return (
    <div className="dialog-backdrop">
      <section className="dialog update-notice" role="dialog" aria-modal="true">
        <h2>
          Fleet Manager v{release.version} is available (you have v{current})
        </h2>
        {release.notes.trim() && <p className="dialog-body notes">{release.notes.trim()}</p>}
        <Check checked={skip} onChange={setSkip}>
          Skip this version
        </Check>
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
