//! Agent binary discovery + version checks (M6).
//!
//! Before a run launches, the app needs to know the chosen agent's CLI is
//! actually installed, so a missing binary becomes a graceful up-front error
//! instead of a run that spawns, dies mid-loop, and leaves an orphan worktree
//! behind. Discovery also surfaces the installed version against the version
//! this tree's adapter was last verified against — the agent CLIs ship weekly
//! and their event schemas drift (PRD Risks), so a version mismatch is worth a
//! visible warning (but never a blocker).
//!
//! Discovery is one `<binary> --version` spawn: a `NotFound` error means the
//! CLI is not on `PATH`; a success yields the version string.

use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

/// A v1 agent's CLI facts: how to invoke it, and the version this tree's
/// adapter was last integration-tested against (see the adapter progress notes).
pub struct AgentSpec {
    /// Stable key used everywhere else (run records, `build_adapter`).
    pub key: &'static str,
    /// Human name for the UI.
    pub display: &'static str,
    /// The binary looked up on `PATH`.
    pub binary: &'static str,
    /// Flag that prints the version.
    pub version_arg: &'static str,
    /// Version the adapter was integration-tested against.
    pub tested_version: &'static str,
    /// Model names the UI may offer for `--model`, in display order. Empty
    /// when the CLI has no fixed set (any string it accepts is passed through).
    pub models: &'static [&'static str],
}

/// The known agents. Keys match `build_adapter` and the stored run `agent`;
/// the tested versions are the ones the adapters were captured/tested against.
pub const KNOWN_AGENTS: &[AgentSpec] = &[
    AgentSpec {
        key: "claude",
        display: "Claude Code",
        binary: "claude",
        version_arg: "--version",
        tested_version: "2.1.201",
        models: &[
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-haiku-4-5-20251001",
        ],
    },
    AgentSpec {
        key: "pi",
        display: "pi",
        binary: "pi",
        version_arg: "--version",
        tested_version: "0.80.3",
        models: &[],
    },
    AgentSpec {
        key: "cursor",
        display: "cursor-agent",
        binary: "cursor-agent",
        version_arg: "--version",
        tested_version: "2026.07.01",
        models: &[],
    },
    AgentSpec {
        key: "codex",
        display: "Codex",
        binary: "codex",
        version_arg: "--version",
        tested_version: "0.154.0",
        models: &[
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
        ],
    },
];

/// Look up an agent spec by key. Accepts the `cursor-agent` alias for `cursor`,
/// mirroring `build_adapter`'s dispatch.
pub fn spec_for(key: &str) -> Option<&'static AgentSpec> {
    let key = if key == "cursor-agent" { "cursor" } else { key };
    KNOWN_AGENTS.iter().find(|a| a.key == key)
}

fn steering_minimum(key: &str) -> Option<[u64; 3]> {
    match key {
        "codex" => Some([0, 154, 0]),
        "pi" => Some([0, 80, 3]),
        _ => None,
    }
}

fn supports_steering(key: &str, version: Option<&str>) -> bool {
    let Some(minimum) = steering_minimum(key) else {
        return false;
    };
    let Some(version) = version else { return false };
    let parts: Vec<_> = version.split('.').map(str::parse::<u64>).collect();
    matches!(parts.as_slice(), [Ok(major), Ok(minor), Ok(patch)] if [*major, *minor, *patch] >= minimum)
}

pub(crate) fn can_steer(key: &str) -> bool {
    let Some(spec) = spec_for(key) else {
        return false;
    };
    let Ok(output) = std::process::Command::new(spec.binary)
        .arg(spec.version_arg)
        .stdin(Stdio::null())
        .output()
    else {
        return false;
    };
    let text = if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    output.status.success()
        && supports_steering(
            key,
            extract_version(&String::from_utf8_lossy(text)).as_deref(),
        )
}

/// The result of discovering one agent's CLI.
#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub key: String,
    pub display: String,
    pub binary: String,
    pub tested_version: String,
    /// Whether the binary was found on `PATH` and ran.
    pub installed: bool,
    pub can_steer: bool,
    /// The detected version (the first version-like token in `--version`
    /// output), if installed and recognized.
    pub version: Option<String>,
    /// `Some(true|false)` once installed: does the detected version match the
    /// version the adapter was tested against? A mismatch is a warning, not a
    /// blocker. `None` when not installed or the version wasn't recognized.
    pub version_matches: Option<bool>,
    /// Human-readable reason when not installed, or a note when the version
    /// output wasn't recognized.
    pub detail: Option<String>,
    /// Model names the UI may offer for `--model`, in display order. Empty
    /// when the CLI has no fixed set. Mirrors `AgentSpec::models`.
    pub models: Vec<String>,
}

/// Discover one agent: run `<binary> --version`. A `NotFound` spawn error means
/// the CLI is not installed; any other spawn error is reported but still leaves
/// `installed: false` (the binary couldn't be run).
pub async fn discover(spec: &AgentSpec) -> AgentStatus {
    let missing = |detail: String| AgentStatus {
        key: spec.key.into(),
        display: spec.display.into(),
        binary: spec.binary.into(),
        tested_version: spec.tested_version.into(),
        installed: false,
        can_steer: false,
        version: None,
        version_matches: None,
        detail: Some(detail),
        models: spec.models.iter().map(|m| m.to_string()).collect(),
    };

    let output = Command::new(spec.binary)
        .arg(spec.version_arg)
        .stdin(Stdio::null())
        .output()
        .await;

    match output {
        Ok(out) if !out.status.success() => missing(format!(
            "'{} --version' exited with {}",
            spec.binary, out.status
        )),
        Ok(out) => {
            // Prefer stdout; some CLIs print the version banner on stderr.
            let stdout = String::from_utf8_lossy(&out.stdout);
            let text = if stdout.trim().is_empty() {
                String::from_utf8_lossy(&out.stderr).into_owned()
            } else {
                stdout.into_owned()
            };
            let version = extract_version(&text);
            let version_matches = version.as_deref().map(|v| v == spec.tested_version);
            AgentStatus {
                key: spec.key.into(),
                display: spec.display.into(),
                binary: spec.binary.into(),
                tested_version: spec.tested_version.into(),
                installed: true,
                can_steer: supports_steering(spec.key, version.as_deref()),
                detail: if steering_minimum(spec.key).is_some()
                    && !supports_steering(spec.key, version.as_deref())
                {
                    Some("live steering unavailable for this version; use notes".into())
                } else {
                    version
                        .is_none()
                        .then(|| "version output not recognized".into())
                },
                version,
                version_matches,
                models: spec.models.iter().map(|m| m.to_string()).collect(),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            missing(format!("'{}' not found on PATH", spec.binary))
        }
        Err(e) => missing(format!("could not run '{}': {e}", spec.binary)),
    }
}

/// Discover all known agents. Four fast `--version` commands run in sequence.
pub async fn discover_all() -> Vec<AgentStatus> {
    let mut out = Vec::with_capacity(KNOWN_AGENTS.len());
    for spec in KNOWN_AGENTS {
        out.push(discover(spec).await);
    }
    out
}

/// Pull the first version-like token (`\d+(\.\d+)+`, e.g. `2.1.201` or the
/// date-shaped `2026.07.01`) out of `--version` output. Returns `None` when no
/// dotted number is present.
fn extract_version(output: &str) -> Option<String> {
    let bytes = output.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                if bytes[i] == b'.' {
                    dots += 1;
                }
                i += 1;
            }
            let tok = &output[start..i];
            // A version has at least one dot and doesn't end on one.
            if dots >= 1 && !tok.ends_with('.') {
                return Some(tok.to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentAdapter;

    #[test]
    fn steering_requires_supported_versions() {
        for (key, older, minimum, newer) in [
            ("codex", "0.153.9", "0.154.0", "0.162.0"),
            ("pi", "0.80.2", "0.80.3", "1.1.0"),
        ] {
            assert!(!supports_steering(key, None));
            assert!(!supports_steering(key, Some("invalid")));
            assert!(!supports_steering(key, Some(older)));
            assert!(supports_steering(key, Some(minimum)));
            assert!(supports_steering(key, Some(newer)));
        }
        assert!(!crate::ClaudeAdapter.can_steer());
        assert!(!crate::CursorAdapter.can_steer());
        for key in ["claude", "cursor", "cursor-agent"] {
            assert!(!supports_steering(key, Some("999.0.0")));
        }
    }

    #[tokio::test]
    async fn discovery_agrees_with_adapters() {
        for (key, adapter) in [
            ("codex", &crate::CodexAdapter as &dyn AgentAdapter),
            ("pi", &crate::PiAdapter),
            ("claude", &crate::ClaudeAdapter),
            ("cursor", &crate::CursorAdapter),
        ] {
            let status = discover(spec_for(key).unwrap()).await;
            assert_eq!(status.can_steer, adapter.can_steer());
        }
    }

    #[test]
    fn extracts_version_from_banners() {
        assert_eq!(
            extract_version("2.1.201 (Claude Code)").as_deref(),
            Some("2.1.201")
        );
        assert_eq!(
            extract_version("pi version 0.80.3").as_deref(),
            Some("0.80.3")
        );
        assert_eq!(
            extract_version("cursor-agent 2026.07.01").as_deref(),
            Some("2026.07.01")
        );
        // Prerelease suffix is dropped at the first non-[0-9.] byte.
        assert_eq!(extract_version("v1.2.3-beta").as_deref(), Some("1.2.3"));
    }

    #[test]
    fn rejects_non_versions() {
        assert_eq!(extract_version("no version here"), None);
        // A bare integer or a trailing dot is not a version.
        assert_eq!(extract_version("version 7"), None);
        assert_eq!(extract_version("saw 2. items"), None);
    }

    #[test]
    fn spec_lookup_and_cursor_alias() {
        assert_eq!(spec_for("claude").map(|s| s.binary), Some("claude"));
        assert_eq!(spec_for("codex").map(|s| s.binary), Some("codex"));
        assert_eq!(spec_for("pi").map(|s| s.binary), Some("pi"));
        // Both the key and the CLI name resolve to the same cursor spec.
        assert_eq!(spec_for("cursor").map(|s| s.binary), Some("cursor-agent"));
        assert_eq!(
            spec_for("cursor-agent").map(|s| s.binary),
            Some("cursor-agent")
        );
        assert!(spec_for("nope").is_none());
    }

    #[tokio::test]
    async fn missing_binary_is_graceful() {
        let spec = AgentSpec {
            key: "ghost",
            display: "Ghost",
            binary: "loopfleet-nonexistent-binary-xyz",
            version_arg: "--version",
            tested_version: "0.0.0",
            models: &[],
        };
        let status = discover(&spec).await;
        assert!(!status.installed);
        assert_eq!(status.version, None);
        assert!(status.detail.unwrap().contains("not found on PATH"));
    }

    // Live discovery against a real CLI. Ignored by default (needs the binary
    // installed); run with `--ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_discovers_claude() {
        let spec = spec_for("claude").unwrap();
        let status = discover(spec).await;
        assert!(status.installed, "claude CLI should be installed");
        assert!(status.version.is_some(), "should detect a version");
    }
}
