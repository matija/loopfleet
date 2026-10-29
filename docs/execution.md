# Task execution

## Write an ordinary PRD

Put `PRD.md` in the repository root, or use the registered `plans/` convention. Git tracking is not required.

Describe the product, shared decisions, and acceptance criteria. Use Markdown checkboxes for tasks. Nested task details can remain nested.

Loopfleet captures the complete plan at launch. Each agent receives that reference plus its selected task. Only the selected task is in scope.

You do not need to repeat Git instructions, progress paths, completion markers, or runner rules in task text.

## Git ownership

The agent changes worktree files and performs task checks. Loopfleet owns staging, snapshots, commits, and merges.

The runner supplies this contract on every pass. Codex also receives it through developer instructions. Claude and pi receive appended system instructions.

The sandbox still denies agent writes to repository Git metadata. Do not grant broader write access to fix a commit denial.

After verification, review the diff and merge the run. Automatic merging remains an explicit app preference. Existing preferences do not change.

## Configure project checks once

An optional `.loopfleet.json` in the registered repository defines external prerequisites and mandatory verification commands.

Your planning agent can create this file once from the existing project scripts. Later PRDs reuse it without extra runner instructions.

Example:

```json
{
  "version": 1,
  "preflight": [
    {
      "name": "Docker daemon available. Start Docker before resuming",
      "command": ["docker", "info"],
      "timeout_seconds": 15
    }
  ],
  "verify": [
    {
      "name": "Frontend checks",
      "command": ["npm", "run", "verify"],
      "timeout_seconds": 300
    },
    {
      "name": "Database acceptance",
      "command": ["npm", "run", "verify:mcp"],
      "timeout_seconds": 1200
    }
  ]
}
```

This example suits Esploro. Other projects use their own commands. Loopfleet includes its own configuration as another example.

Commands are argument arrays, not shell strings. Each command runs from the task worktree, inside the same sandbox as the agent.

Shell syntax requires an explicit shell command, such as `["sh", "-c", "your command"]`. Commands receive the same writable Cargo cache as sandboxed agents.

Timeouts range from 1 through 3,600 seconds. The default is 300 seconds. A configuration can contain at most 32 checks.

The runner captures the configuration before agent execution. Agent edits cannot remove required checks from that run. They apply only to later launches.

Only external prerequisites belong in `preflight`. An agent can install project dependencies in its worktree during implementation. Missing worktree dependencies are not necessarily external blockers.

Start host services, such as Docker Desktop or Colima, outside the agent sandbox. Loopfleet does not start host VMs or grant access automatically.

An available Docker daemon does not prove all container operations work. Required verification still runs inside the sandbox and must succeed.

## Outcomes

- **Ready for review:** The agent supplied a valid task result, runner verification passed, and Loopfleet captured a snapshot.
- **Blocked:** An external prerequisite prevents work or required verification. The reason includes a recovery action. Existing work remains available.
- **Failed:** Implementation, verification, result recording, or snapshot capture failed. Useful snapshots remain available for repair.
- **Stopped:** You stopped the run.
- **Rate-limited:** The agent reached its provider limit. Existing resume behavior applies.

A preflight failure blocks the run before an agent pass starts. It does not consume repeated agent attempts.

For blocked work, restore the prerequisite and choose **Resume with Agent**. Loopfleet creates a linked run with the previous available changes. It repeats preflight and verification.

Blocked worktrees are excluded from automatic age-based cleanup. Explicit project removal or worktree cleanup can still remove them.

Automatic plan continuation stops at a blocked task. It does not skip that task and start later work.

Failed and blocked runs cannot merge. An existing completed run keeps its historical status. The stronger verification contract applies to new runs.

## Completion and evidence

Agents report a per-pass JSON result instead of relying on a completion marker. Loopfleet supplies its exact path in the prompt.

A ready result contains a short summary and executable checks. A blocked result contains the external prerequisite and recovery action.

The runner executes configured verification commands and agent-reported checks. Identical command arrays run once, with the configured check taking precedence.

Without project configuration, the runner executes the agent-reported checks. A ready result with no checks cannot complete a run.

The runner records command names, argument arrays, exit codes, bounded output, and pass/fail results. Failed checks return as feedback to the next permitted pass.

A plain completion marker cannot complete a run. A skipped or failed check cannot count as passing.

Check selection without project configuration still depends on the agent. Configure mandatory project checks when you need consistent acceptance gates.

A passing command proves its exit result, not the quality of every requirement or test. Human review remains necessary.

Per-run files include the captured `plan.md`, captured `execution.json`, per-pass agent results, and runner verification evidence. They live outside the repository.

These files can contain command output, SQL, or other sensitive project details. Do not print secrets in checks.

## Local verification

Run `npm run verify` from the Loopfleet repository root.

The command builds the frontend, runs frontend tests and interface checks, runs workspace Rust tests, and runs Clippy.

Live-provider and macOS sandbox tests remain separate where their existing test definitions require explicit execution. Do not count ignored tests as passing.
