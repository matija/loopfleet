use serde::{Deserialize, Serialize};

pub use crate::event::Delivery;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tap {
    pub id: String,
    pub text: String,
    pub created_at_ms: i64,
}

pub fn render_notes(taps: &[Tap]) -> Option<String> {
    (!taps.is_empty()).then(|| {
        format!(
            "## Corrections from the human\n\n{}",
            taps.iter()
                .map(|tap| tap.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    })
}

pub fn route(can_steer: bool, pass_in_flight: bool, steering_enabled: bool) -> Delivery {
    if can_steer && pass_in_flight && steering_enabled {
        Delivery::Steered
    } else {
        Delivery::Queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_notes_are_absent() {
        assert_eq!(render_notes(&[]), None);
    }

    #[test]
    fn notes_label_human_corrections_and_preserve_order_and_text() {
        let taps = [
            "Use the existing API.\nKeep its name.",
            "Keep the output concise.",
        ]
        .map(|text| Tap {
            id: String::new(),
            text: text.into(),
            created_at_ms: 0,
        });
        assert_eq!(
            render_notes(&taps),
            Some("## Corrections from the human\n\nUse the existing API.\nKeep its name.\n\nKeep the output concise.".into())
        );
        assert_eq!(
            render_notes(&taps[..1]),
            Some("## Corrections from the human\n\nUse the existing API.\nKeep its name.".into())
        );
    }

    #[test]
    fn routes_all_boolean_combinations() {
        for (can_steer, pass_in_flight, steering_enabled, expected) in [
            (false, false, false, Delivery::Queued),
            (false, false, true, Delivery::Queued),
            (false, true, false, Delivery::Queued),
            (false, true, true, Delivery::Queued),
            (true, false, false, Delivery::Queued),
            (true, false, true, Delivery::Queued),
            (true, true, false, Delivery::Queued),
            (true, true, true, Delivery::Steered),
        ] {
            assert_eq!(route(can_steer, pass_in_flight, steering_enabled), expected);
        }
    }
}
