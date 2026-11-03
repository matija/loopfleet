import { BoxIcon } from "./Icon";

export function SandboxBoundaryPanel() {
  return (
    <section className="panel" aria-labelledby="sandbox-heading">
      <div className="panel__head">
        <BoxIcon size={16} className="icon panel__icon" />
        <h3 id="sandbox-heading">Sandbox boundary</h3>
      </div>
      <p className="panel__lead">
        Agents run with their own permission prompts disabled.
      </p>

      <div className="sandbox-rules">
        <div className="sandbox-rule">
          <span className="sandbox-rule__badge sandbox-rule__badge--confined">
            Confined
          </span>
          <div className="sandbox-rule__text">
            <strong>Writes</strong>
            <p>
              The run’s worktree, progress directory, and configured extra paths.
              Loopfleet owns commits. Verification uses executable tests.
            </p>
          </div>
        </div>

        <div className="sandbox-rule">
          <span className="sandbox-rule__badge sandbox-rule__badge--open">
            Open
          </span>
          <div className="sandbox-rule__text">
            <strong>Reads &amp; network</strong>
            <p>
              Unrestricted. Agents can read files accessible to your account,
              including credentials, and send data over the network.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
