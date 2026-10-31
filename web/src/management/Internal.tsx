import type { InternalRemote } from "../api/generated/InternalRemote";
import { ago } from "../api/schema";
import { Button } from "../ui/Button";
import { ErrorLine } from "../ui/ErrorLine";
import { ListRow } from "../ui/ListRow";
import { Select } from "../ui/Select";
import type { InternalApi } from "./internal";

// A port of crates/fleet-gui/src/views/management/internal.rs's `internal_section`: a header that opens on the
// branches, the files of one, and one file's text with the commit that last touched it.

export function Internal({ internal, now }: { internal: InternalApi; now: number }) {
  const { st } = internal;
  return (
    <section className="internal">
      <button type="button" className="disclosure" aria-expanded={st.open} onClick={st.open ? internal.close : internal.open}>
        Internal resources
      </button>
      {st.open && (
        <div className="internal-body">
          <div className="product-actions">
            <p className="note">Designs, plans and decisions the orchestrators keep for this product. Read only.</p>
            <Button className="ghost" onClick={internal.refresh}>
              Refresh
            </Button>
          </div>
          {st.error !== null && <ErrorLine text={st.error} />}
          {st.branches !== null && (
            <>
              <Remote remote={st.remote} />
              {st.branches.length === 0 ? (
                <p className="note">No internal resources yet</p>
              ) : (
                <>
                  <Select
                    label="Internal branch"
                    value={st.branch ?? ""}
                    options={st.branches.map((b) => ({ value: b, label: b }))}
                    onChange={internal.pickBranch}
                  />
                  {st.files !== null &&
                    (st.files.length === 0 ? (
                      <p className="note">No files on this branch</p>
                    ) : (
                      <ul className="internal-files">
                        {st.files.map((f) => (
                          <li key={f}>
                            <ListRow name={f} selected={f === st.file} onSelect={() => internal.pickFile(f)}>
                              <span className="list-row-name internal-path">{f}</span>
                            </ListRow>
                          </li>
                        ))}
                      </ul>
                    ))}
                  {st.text !== null && (
                    <>
                      {st.last && <p className="note">{`${st.last.subject} · ${st.last.author} · ${ago(now, st.last.at)}`}</p>}
                      <pre className="internal-text">{st.text}</pre>
                    </>
                  )}
                </>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** egui's `remote_line`: where the notes are pushed, what is not, and each branch parked for a hand merge. */
function Remote({ remote }: { remote: InternalRemote }) {
  const n = remote.unpushed;
  return (
    <>
      <p className="note">{remote.url !== null ? `Remote: ${remote.url}` : "Local only"}</p>
      {n > 0 && <p className="note">{`${n} ${n === 1 ? "commit" : "commits"} not pushed`}</p>}
      {remote.parked.map((b) => (
        <p key={b} className="note">{`Unpushed notes parked on ${b}; merge it by hand`}</p>
      ))}
      {remote.error !== null && <ErrorLine text={remote.error} />}
    </>
  );
}
