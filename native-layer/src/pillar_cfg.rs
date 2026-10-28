//! Settings of the M182 Eternal Pillar of Flames module (`m182_pillar.json`, its own file).

use serde::Deserialize;

pub const CONFIG_FILE: &str = "m182_pillar.json";

/// Template of the zone the Eternal Pillar Of Flames creates (`Zone_Spell_Fire_Pillar_4`).
/// Radius of the damage capsule of `Zone_Spell_Fire_Pillar_4`.
pub const BASE_RADIUS_M: f32 = 3.0;

/// `VFX_Scavenger_Camp_Bonfire`, a looping fire effect.
pub const VISUAL_TEMPLATE: &str = "d0d55bba-7951-4213-8c2d-f8efb416fa2a";

pub const ETERNAL_ZONE_TEMPLATE: &str = "a77023ba-4010-466b-8554-61fd3e9a621b";

/// Item ids of the veil beam spells (`Ammo_T7_Spell_Fog_Beam_Channel_lvl45_Eternal` and
/// `Ammo_T5_Spell_Fog_Beam_Channel_lvl30`).
pub const BEAM_ITEMS: [u32; 2] = [803_538_053, 2_153_855_871];

/// `keen::actor::StateFlag::InRangeCombatShooting`: set while a channelled spell is held.
pub const FLAG_SHOOTING: u32 = 27;

pub const BEAM_EFFECT: &str = "VFX_Player_Projectile_Wand_Heavy_Piercing_Impact_Fog";

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
    /// Adds game fire effects (seen by every player) around the zone, because the pillar's own
    /// flames are drawn by each client at a fixed size.
    pub visual: bool,
    /// Effect templates (GUIDs) used for the visual field.
    pub visual_templates: Vec<String>,
    /// Size of each effect.
    pub visual_scale: f32,
    /// Effects placed in a circle around the centre (0 = only the centre one).
    pub visual_ring: u32,
    /// When an administrator and another player both stand near the cast, the nearest one decides
    /// (default false: the pillar stays normal, because the caster cannot be told apart).
    pub nearest_decides: bool,
    /// Draws big game effects along the veil beam while it is channelled (seen by every player).
    pub beam: bool,
    /// Effects (names of the VFX catalog or GUIDs) placed along the beam.
    pub beam_effects: Vec<String>,
    /// Size of each effect of the beam.
    pub beam_scale: f32,
    /// Effects along the beam.
    pub beam_spots: u32,
    /// Length covered by the effects, in meters.
    pub beam_length_m: f32,
    /// Milliseconds between two refreshes of the effects.
    pub beam_every_ms: u64,
}

impl Default for PillarConfig {
    fn default() -> Self {
        Self { mode: "off".into(), radius_m: 15.0, duration_s: 15.0, max_dumps: 12, set_duration: true, admin_only: false, visual: true, visual_templates: vec![VISUAL_TEMPLATE.into()], visual_scale: 4.0, visual_ring: 8, nearest_decides: false, beam: false, beam_effects: vec![BEAM_EFFECT.into()], beam_scale: 3.0, beam_spots: 6, beam_length_m: 18.0, beam_every_ms: 500 }
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

    /// Radius of the circle of effects: inside the damage radius.
    pub fn ring_radius_m(&self) -> f32 {
        (self.radius_m * 0.6).clamp(1.0, 30.0)
    }

    /// Points along the beam from `origin`, pushed along `facing` (horizontal) at chest height.
    pub fn beam_points(&self, origin: (i64, i64, i64), facing: (f64, f64)) -> Vec<(i64, i64, i64)> {
        const METERS: f64 = 4_294_967_296.0;
        let spots = self.beam_spots.clamp(1, 12);
        let length = self.beam_length_m.clamp(3.0, 20.0) as f64;
        let first = 2.0f64.min(length);
        (0..spots)
            .map(|i| {
                let d = if spots == 1 { length } else { first + (length - first) * i as f64 / (spots - 1) as f64 };
                (
                    origin.0 + (facing.0 * d * METERS) as i64,
                    origin.1 + (1.2 * METERS) as i64,
                    origin.2 + (facing.1 * d * METERS) as i64,
                )
            })
            .collect()
    }

    /// Milliseconds an effect of the beam lives: a little longer than the refresh time.
    pub fn beam_life_ms(&self) -> u64 {
        self.beam_every_ms.clamp(200, 2000) + 150
    }

    pub fn beam_size(&self) -> f32 {
        if self.beam_scale.is_finite() { self.beam_scale.clamp(0.5, 10.0) } else { 3.0 }
    }

    pub fn wanted(&self) -> bool {
        self.observing() || self.active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visual_field_has_sane_defaults() {
        let c = PillarConfig::default();
        assert!(c.visual && c.visual_ring == 8 && c.visual_scale == 4.0);
        assert!(super::super::config::parse_guid(&c.visual_templates[0]).is_some());
        assert_eq!(c.ring_radius_m(), 9.0);
    }

    #[test]
    fn every_player_gets_the_big_pillar_unless_limited_to_admins() {
        assert!(!PillarConfig::default().admin_only);
        assert!(PillarConfig::parse(r#"{"admin_only":true}"#).admin_only);
    }

    #[test]
    fn mixed_casters_stay_normal_by_default() {
        assert!(!PillarConfig::default().nearest_decides);
        assert!(PillarConfig::parse(r#"{"nearest_decides":true}"#).nearest_decides);
    }

    #[test]
    fn off_unless_asked() {
        assert!(!PillarConfig::default().wanted());
        assert!(!PillarConfig::parse("not json").wanted());
        assert!(PillarConfig::parse(r#"{"mode":"Observe"}"#).observing());
        let on = PillarConfig::parse(r#"{"mode":"on","radius_m":12}"#);
        assert!(on.active() && on.radius_m == 12.0 && on.duration_s == 15.0);
    }

    #[test]
    fn radius_and_time_are_converted_and_limited() {
        let c = PillarConfig::default();
        assert_eq!(c.scale(), 5.0);
        assert_eq!(c.duration_ns(), 15_000_000_000);
        let wild = PillarConfig::parse(r#"{"radius_m":900,"duration_s":9999}"#);
        assert_eq!(wild.scale(), 10.0);
        assert_eq!(wild.duration_ns(), 120_000_000_000);
        assert_eq!(PillarConfig::parse(r#"{"radius_m":0.1}"#).scale(), 1.0);
    }

    #[test]
    fn the_beam_defaults_are_sane() {
        let c = PillarConfig::default();
        assert!(!c.beam && c.beam_spots == 6 && c.beam_every_ms == 500);
        assert_eq!(c.beam_life_ms(), 650);
        assert_eq!(c.beam_size(), 3.0);
        assert!(PillarConfig::parse(r#"{"beam":true}"#).beam);
        assert_eq!(PillarConfig::parse(r#"{"beam_scale":99}"#).beam_size(), 10.0);
        assert_eq!(PillarConfig::parse(r#"{"beam_every_ms":1}"#).beam_life_ms(), 350);
    }

    #[test]
    fn the_beam_points_run_ahead_of_the_caster() {
        const M: f64 = 4_294_967_296.0;
        let c = PillarConfig::default();
        let pts = c.beam_points((0, 0, 0), (0.0, 1.0));
        assert_eq!(pts.len(), 6);
        assert_eq!(pts[0].2, (2.0 * M) as i64);
        assert_eq!(pts[5].2, (18.0 * M) as i64);
        assert!(pts.iter().all(|p| p.0 == 0 && p.1 == (1.2 * M) as i64));
        assert!(pts.windows(2).all(|w| w[0].2 < w[1].2));
        let one = PillarConfig::parse(r#"{"beam_spots":1,"beam_length_m":10}"#).beam_points((0, 0, 0), (1.0, 0.0));
        assert_eq!(one, vec![((10.0 * M) as i64, (1.2 * M) as i64, 0)]);
        let wild = PillarConfig::parse(r#"{"beam_spots":500,"beam_length_m":900}"#).beam_points((0, 0, 0), (0.0, 1.0));
        assert_eq!(wild.len(), 12);
        assert_eq!(wild[11].2, (20.0 * M) as i64);
    }

    #[test]
    fn the_beam_effect_and_items_are_known() {
        assert!(BEAM_ITEMS.contains(&803_538_053));
        assert_eq!(FLAG_SHOOTING, 27);
        assert!(BEAM_EFFECT.starts_with("VFX_"));
    }

    #[test]
    fn the_zone_template_is_a_guid() {
        assert!(super::super::config::parse_guid(ETERNAL_ZONE_TEMPLATE).is_some());
    }
}
