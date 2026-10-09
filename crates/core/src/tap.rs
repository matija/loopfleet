use serde::{Deserialize, Serialize};

pub use crate::event::Delivery;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tap {
    pub id: String,
    pub text: String,
    pub created_at_ms: i64,
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
