import { Page } from "./Page";

/** A tab whose port lands later: it is listed, as egui's tests count every tab, and says so. */
export function NotYet() {
  return (
    <Page>
      <p className="note">not on the web yet</p>
    </Page>
  );
}
