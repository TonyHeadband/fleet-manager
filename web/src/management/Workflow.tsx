// A port of crates/fleet-gui/src/views/management/workflow.rs: what `.fleet/workflow.yml` is. Read-only until the
// editor exists.

/** The two-step example the paragraph above it describes. */
const EXAMPLE = `ignore: [docs/**, "**/*.md"]
steps:
  fmt:
    run: cargo fmt --all --check
  daemon:
    run: cargo test -p fleet-daemon
    paths: [crates/fleet-daemon/**]`;

export function Workflow() {
  return (
    <div className="management-page">
      <div className="workflow">
        <h2>Workflow</h2>
        <p>
          Each repo lists its checks in .fleet/workflow.yml on its base branch. Before an orchestrator opens a pull request, the app runs
          the steps whose paths the task changed (every step when a changed file is not covered). Agents cannot change it: it is read from
          the remote.
        </p>
        <pre className="example">{EXAMPLE}</pre>
        <p className="note">An editor for this file is coming. For now, add the file to the repo and push it.</p>
      </div>
    </div>
  );
}
