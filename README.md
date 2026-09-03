# loopfleet <img src="src-tauri/icons/icon.png" width="36" height="36" alt="">

Agent cockpit for spec-and-loop driven development.

<p align="center">
  <img src="docs/screenshot.png" alt="loopfleet main view — fleet of runs across harnesses and models" width="900">
  <br><em>Fleet view — every scheduled run, its harness, model, and status at a glance.</em>
</p>

<p align="center">
  <img src="docs/screenshot-diff.png" alt="loopfleet diff review" width="900">
  <br><em>Diff review — inspect exactly what each agent produced before accepting a task.</em>
</p>

Write a PRD, break it into tasks, and let coding agents loop on them until every task is accepted. 🚀

loopfleet is a native macOS app for **scheduled and continuous task execution**. Configure runs with your own settings — choose a harness (pi, Claude, Cursor, …), pick a model, set the cadence — and agents work through your task list around the clock, on repeat or until everything is accepted.

Context is kept deliberately small per run, so each agent stays focused on its current task instead of drowning in accumulated history. The app keeps the plan, the runs, and the resulting diffs in one place: scan the fleet, review what changed, and accept or reject tasks without losing the thread.

## Download

<!-- download-links:start -->
**Download loopfleet 0.1.13** —
[Apple Silicon](https://github.com/matija/loopfleet/releases/download/0.1.13/loopfleet_0.1.13_aarch64.dmg)
· [Intel](https://github.com/matija/loopfleet/releases/download/0.1.13/loopfleet_0.1.13_x64.dmg)
<!-- download-links:end -->

Signed and notarized `.dmg` builds; the app updates itself from there.
Older builds: [all releases](https://github.com/matija/loopfleet/releases).

> WIP.

## Build from source 🛠️

See [`build/README.md`](build/README.md).
