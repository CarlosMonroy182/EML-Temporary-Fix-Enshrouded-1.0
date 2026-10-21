# M182 VFX - notes

- Mod `spawner-mods/m182_vfx` (EML Lua, capability patch). Config `src/m182_vfx_config.lua` (efecto, color, intensidad, hechizos).
- Staff spells are ammo items (`keen::ItemInfo`); `visualEntity` points to `VFX_Ability_Casting_*` templates, shown while aiming/charging. Each has `keen::ecs::VfxComponentResource.vfx`. The mod retargets that guid (same mechanism as the working M182 Fire Aura mod on `1_Player`).
- Intensity multiplies `particles[].maxCount` of the chosen VfxResource in place; colors write `defaultParameterData` slots of type `keen::VfxEffectParameterColor` (1392743968, uint32 0xAABBGGRR). In place = shared by every use of that effect.
- VFX parameter names are FNV-1a32 (`color`, `light_radius`, `light_intensity`, `light_enabled`, `spell_charge_duration` = 0x3f7047a3, set by game code).
- Dead end: `SpecialAbilityChargeFx` notifier `ba663a7a-...` on player templates. Changing it (even byte-identical except color) never showed anything in game; it is not the visible charge aura.
- Offline check: `emm run --patch -g <dir with enshrouded.kfc, .kfc.bak, .kfc_resources, .exe, mods/>` then `kfc-parser unpack -f <guid>_<typehash>_0` (TemplateResource 39768775, VfxResource e64ee036, NotifierResource 88d6b534).
- Raw resource bytes: `cargo build --release -p cli-legacy --example dumpres`, then `dumpres <game dir> <guid>_<typehash>_<part>`.
- Not confirmed in the real game: spell-visual retarget, intensity, colors. Melee charged attack visuals not covered.
- Custom effects: particle programs are compiled GPU code in .dat content (no public tool). Feasible custom work: own textures via `image.encode_texture` + `create_content` on existing particle systems, colors, counts.

## Native M182 VFX (charge aura + charged release burst)
- `native-layer/src/vfx.rs` (copy in `eml-fork/.../native/vfx.rs`). Own config `m182_vfx.json` and own log `m182_vfx.log`; does nothing when the json is missing. No panel code involved.
- Detection: player `keen::ecs::CombatTargetActor` (24 bytes: abilityMask u32, pad, combinedState u64 `keen::actor::StateFlag`, lastRangedWeaponUsage). Found by heuristic (padding 0, IsReadyForGameplay bit 50). Charging = HeavyAttack (29) or InRangeCombatPreparing (26); release = end of a charge longer than `min_charge_ms`.
- Effects: VFX_ entity templates (CurrentTransform + LifeTime, no damage components) created on the server from the spawner's create hook (`spawner::before` drains `vfx::run`). CurrentTransform scale (+40 float3) = `scale`. Built-in name->guid catalog in vfx.rs.
- Test key (host only): `test_key` (default F9) bursts the release effects on the host.
- Not confirmed in game: heuristic picks the right component, scale applies to VFX, effects replicate to clients.
