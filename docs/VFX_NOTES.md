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

## Elements and weapon position (Oct 21)
- SlotAttachment is client_only: server-created effects cannot be glued to a weapon or bone. Server-side bone positions come from the player's `ServerAnimationGraphSlotComponent` (1760 bytes: slot mask u64, origin WorldPosition, 36 x float3x4 local slots indexed by AttachmentSlot; HandR = 8). Blade tip = HandR + axis[BLADE_AXIS] * BLADE_LENGTH (axis still being confirmed with F10 logs).
- Element of the item in use: staff spell from `NetworkRangedWeapon` (+68 ammunition item, +56 weapon item), melee weapon from `Equipment` (+16 main-hand impact item). Damage columns come from `export/m182_items.csv`; `element_of` picks the strongest magic damage, blunt-only = bone, no damage = arcane, else physical.
- Effects per element are built in (`staff_effects`, `blade_effect`); `"auto": true` (default) ignores the effect lists in the json.

## Hand glow (Oct 21, later)
- Melee charge: the Lua mod `m182_vfx` (client and server) clones `VFX_Debuff_Poison_GemPerk_DamageOverTime` (has ServerSlotAttachment + LifeTime + CurrentTransform) into 9 templates `182e1e00-0000-4000-8000-00000000000N` (N = element 1..9) with the element's glow VfxResource and its default parameters.
- The native module creates them with overrides CurrentTransform + ServerSlotAttachment (hash 0x430f162d, 112 bytes: SlotAttachmentBase 48 + attachmentTransform + updated; slot HandR, attachedToEntityId = player) + LifeTime 450 ms, renewed every 300 ms while charging.
- Staff: arcane (mana drain) uses VFX_Spell_ManaDrain_Mana_OnPlayer + VFX_Resource_Increase_Mana at 1.25x; shroud uses only the fog evade impact at 0.6x.

## Walking aura (Oct 21)
- Per player, server side. Crouch (InSneakMode bit 21 rising edge) = on/off; victory emote (PlayerInput 1472 bytes, triggerEmoteAction at +696 version, +700 id; victory = 1355035795) = next effect. Fire aura and fire staff use the burning glow template glued to the chest (needs the Lua mod on the viewer); mana and holy light override LifeTime to 1.2 s.
