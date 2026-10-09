use serde::{Deserialize, Serialize};

pub use crate::event::Delivery;

pub const MAX_TAP_TEXT_CHARS: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TapText(String);

impl TryFrom<String> for TapText {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.trim().is_empty() {
            Err("tap text must not be empty or whitespace-only")
        } else if text.chars().count() > MAX_TAP_TEXT_CHARS {
            Err("tap text must not exceed 16384 characters")
        } else {
            Ok(Self(text))
        }
    }
}

impl TapText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<TapText> for String {
    fn from(text: TapText) -> Self {
        text.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tap {
    pub id: String,
    pub text: TapText,
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

pub(crate) fn acknowledged(result: Option<Result<(), crate::adapter::AdapterError>>) -> Delivery {
    match result {
        Some(Ok(())) => Delivery::Steered,
        Some(Err(_)) => Delivery::Queued,
        None => Delivery::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_whitespace_only_text() {
        for text in ["", " \t\r\n", "\u{2003}\u{00a0}"] {
            assert!(TapText::try_from(text.to_owned()).is_err());
        }
    }

    #[test]
    fn enforces_character_limit_without_changing_text() {
        for text in [
            "  Keep this.\n".to_owned(),
            "a".repeat(MAX_TAP_TEXT_CHARS - 1),
            "🦀".repeat(MAX_TAP_TEXT_CHARS),
        ] {
            assert_eq!(TapText::try_from(text.clone()).unwrap().as_str(), text);
        }
        assert!(TapText::try_from("a".repeat(MAX_TAP_TEXT_CHARS + 1)).is_err());
        assert!(TapText::try_from("🦀".repeat(MAX_TAP_TEXT_CHARS + 1)).is_err());
    }

    #[test]
    fn deserialization_enforces_text_validation_and_preserves_wire_shape() {
        for text in [
            String::new(),
            " \n\u{2003}".into(),
            "a".repeat(MAX_TAP_TEXT_CHARS + 1),
        ] {
            let value = serde_json::json!({"id": "tap", "text": text, "created_at_ms": 1});
            assert!(serde_json::from_value::<Tap>(value).is_err());
        }
        let value = serde_json::json!({"id": "tap", "text": "🦀".repeat(MAX_TAP_TEXT_CHARS), "created_at_ms": 1});
        let tap: Tap = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(tap).unwrap(), value);
    }

    #[test]
    fn empty_notes_are_absent() {
        assert_eq!(render_notes(&[]), None);
    }

    #[test]
    fn notes_label_human_corrections_and_preserve_order_and_text() {
        let taps = [
            "  Use the existing API. 🦀\nKeep its name.\n",
            "Keep the output concise.",
        ]
        .map(|text| Tap {
            id: String::new(),
            text: text.to_owned().try_into().unwrap(),
            created_at_ms: 0,
        });
        assert_eq!(
            render_notes(&taps),
            Some("## Corrections from the human\n\n  Use the existing API. 🦀\nKeep its name.\n\n\nKeep the output concise.".into())
        );
        assert_eq!(
            render_notes(&taps[..1]),
            Some(
                "## Corrections from the human\n\n  Use the existing API. 🦀\nKeep its name.\n"
                    .into()
            )
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
            assert_eq!(
                route(can_steer, pass_in_flight, steering_enabled),
                expected,
                "can_steer={can_steer}, pass_in_flight={pass_in_flight}, steering_enabled={steering_enabled}"
            );
        }
    }
}
