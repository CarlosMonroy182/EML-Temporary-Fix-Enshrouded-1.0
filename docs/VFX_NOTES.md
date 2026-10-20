# M182 VFX - notes

- Mod `spawner-mods/m182_vfx` (EML Lua, capability patch). Config `src/m182_vfx_config.lua` (efecto, color, colores).
- Charge aura: player templates `1_Player`, `1_Player_AG2`, `AutomatedPlayer` have `keen::ecs::SpecialAbilityChargeFx` (server_only) with notifier `ba663a7a-ba30-4ed3-8ae8-c6ec1d440304` (`keen::NotifierResource`). Its `defaultVfx` is `8822f231-cfdc-493e-94da-dcb68762ef11`, attached to slot Root.
- The mod sets `defaultVfx` to another `keen::VfxResource` and copies that VFX's `defaultParameterData` into `defaultVfxInitParameters` (16 x uint32).
- VFX parameter names are FNV-1a32 hashes (`color` = 1031692888). Parameter type `keen::VfxEffectParameterColor` = 1392743968; value is uint32 0xAABBGGRR.
- VFX names come from templates: `keen::ecs::VfxComponentResource.vfx` (guid of the VfxResource) in `VFX_Ability_Casting_*`, `Explosion_*`, etc.
- Offline check: `emm run --patch -g <dir with enshrouded.kfc, .kfc.bak, .kfc_resources, .exe, mods/>` then `kfc-parser unpack -f ba663a7a-ba30-4ed3-8ae8-c6ec1d440304_88d6b534_0`.
- Not confirmed in the real game: look of each effect on the player Root, that looping effects stop with the charge, server-only install effect.
