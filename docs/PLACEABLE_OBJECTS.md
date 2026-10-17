# Placeable objects (world props) – reference for future weapon VFX mods

- `export/m182_objects.csv` (name,guid,components) lists every entity template (~11,700) with its first 14 component names.
- Template name prefixes: `LootPickup_*` (ground pickups), `LootChest_*`/`LootContainer_*`, `Prop_Decoration_*`, `Prop_Furniture_*`, `Prop_Destructible_*`, `VFX_*`/`Explosion_*`/`Projectile_*`/`Attack_*` (effects), `Visuals_*` (weapon models), `Trap_*`, `NPC_*`, `Animal_*`.
- Spawn an entity: entity-create function with descriptor (template guid at +4, component override array at +0x18, count at +0x20); component entries are (FNV-1a32 of `keen::ecs::<Name>`, data ptr, size). `CurrentTransform` size 56.
- Item pickups: `Base_LootPickup_RandomLoot_Generic` + `keen::ecs::PickupItem` override (28 bytes: itemId u32, item guid 16 B, level u32, rarity u8 @24, amount u16 @26).
- Effects for weapons: VFX templates (`VFX_*`, `Explosion_*`) can be spawned at the player; attaching them to weapons needs an attach component (not solved yet).
