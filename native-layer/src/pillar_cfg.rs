//! Settings of the M182 Eternal Pillar of Flames module (`m182_pillar.json`, its own file).

use serde::Deserialize;

pub const CONFIG_FILE: &str = "m182_pillar.json";

/// Template of the zone the Eternal Pillar Of Flames creates (`Zone_Spell_Fire_Pillar_4`).
pub const ETERNAL_ZONE_TEMPLATE: &str = "a77023ba-4010-466b-8554-61fd3e9a621b";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct PillarConfig {
    /// "off" (default), "observe" (only writes what it sees) or "on".
    pub mode: String,
    pub radius_m: f32,
    pub duration_s: f32,
    pub max_dumps: u32,
}

impl Default for PillarConfig {
    fn default() -> Self {
        Self { mode: "off".into(), radius_m: 15.0, duration_s: 25.0, max_dumps: 12 }
    }
}

impl PillarConfig {
    pub fn parse(text: &str) -> PillarConfig {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn observing(&self) -> bool {
        self.mode.eq_ignore_ascii_case("observe")
    }

    pub fn active(&self) -> bool {
        self.mode.eq_ignore_ascii_case("on")
    }

    pub fn wanted(&self) -> bool {
        self.observing() || self.active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_unless_asked() {
        assert!(!PillarConfig::default().wanted());
        assert!(!PillarConfig::parse("not json").wanted());
        assert!(PillarConfig::parse(r#"{"mode":"Observe"}"#).observing());
        let on = PillarConfig::parse(r#"{"mode":"on","radius_m":12}"#);
        assert!(on.active() && on.radius_m == 12.0 && on.duration_s == 25.0);
    }

    #[test]
    fn the_zone_template_is_a_guid() {
        assert!(super::super::config::parse_guid(ETERNAL_ZONE_TEMPLATE).is_some());
    }
}
