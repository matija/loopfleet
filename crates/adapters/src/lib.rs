//! loopfleet adapters: the `AgentAdapter` trait and per-agent implementations
//! (Claude Code, pi, cursor-agent) that normalize each agent's stream into the
//! shared event enum. Implemented in M1.
//!
//! An adapter's only job is to turn one agent's native transport into the
//! [`NormalizedEvent`] vocabulary. Everything downstream consumes only that
//! enum and never learns which agent produced it.
//!
//! v1 is headless-only: [`AgentAdapter::start_run`] is real; `open_session`
//! stays in the signature (per the PRD's frozen trait) but every v1 adapter
//! returns [`AdapterError::SessionsUnsupported`] — interactive sessions land in
//! M5.

mod claude;
mod cursor;
mod pi;
mod stub;
pub use claude::ClaudeAdapter;
pub use cursor::CursorAdapter;
pub use pi::PiAdapter;
pub use stub::StubAdapter;

// The `AgentAdapter` trait and its launch/handle types live in `core` — next to
// the run loop that drives them — so `core` can compose over a `&dyn AgentAdapter`
// without depending on this crate (which depends on `core`, so the reverse would
// be a cycle). The concrete adapters above `impl` that trait; it is re-exported
// here so `crate::AgentAdapter` (and friends) resolve for them and for existing
// consumers of this crate.
pub use loopfleet_core::{
    AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed,
};
