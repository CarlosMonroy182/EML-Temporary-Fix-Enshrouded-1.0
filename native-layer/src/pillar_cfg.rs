//! Settings of the M182 Eternal Pillar of Flames module (`m182_pillar.json`, its own file).

use serde::Deserialize;

pub const CONFIG_FILE: &str = "m182_pillar.json";

/// Template of the zone the Eternal Pillar Of Flames creates (`Zone_Spell_Fire_Pillar_4`).
/// Radius of the damage capsule of `Zone_Spell_Fire_Pillar_4`.
pub const BASE_RADIUS_M: f32 = 3.0;

pub const ETERNAL_ZONE_TEMPLATE: &str = "a77023ba-4010-466b-8554-61fd3e9a621b";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct PillarConfig {
    /// "off" (default), "observe" (only writes what it sees) or "on".
    pub mode: String,
    pub radius_m: f32,
    pub duration_s: f32,
    pub max_dumps: u32,
    /// Also extends the life of the zone (adds a LifeTime to what the game creates).
    pub set_duration: bool,
    /// Only enlarge pillars cast by a player with the kick/ban permission.
    pub admin_only: bool,
}

impl Default for PillarConfig {
    fn default() -> Self {
        Self { mode: "off".into(), radius_m: 15.0, duration_s: 25.0, max_dumps: 12, set_duration: true, admin_only: true }
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

    /// Uniform scale that turns the 3 m radius of the zone into `radius_m`.
    pub fn scale(&self) -> f32 {
        (self.radius_m / BASE_RADIUS_M).clamp(1.0, 10.0)
    }

    pub fn duration_ns(&self) -> u64 {
        (self.duration_s.clamp(1.0, 120.0) as f64 * 1e9) as u64
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
    fn radius_and_time_are_converted_and_limited() {
        let c = PillarConfig::default();
        assert_eq!(c.scale(), 5.0);
        assert_eq!(c.duration_ns(), 25_000_000_000);
        let wild = PillarConfig::parse(r#"{"radius_m":900,"duration_s":9999}"#);
        assert_eq!(wild.scale(), 10.0);
        assert_eq!(wild.duration_ns(), 120_000_000_000);
        assert_eq!(PillarConfig::parse(r#"{"radius_m":0.1}"#).scale(), 1.0);
    }

    #[test]
    fn the_zone_template_is_a_guid() {
        assert!(super::super::config::parse_guid(ETERNAL_ZONE_TEMPLATE).is_some());
    }
}
