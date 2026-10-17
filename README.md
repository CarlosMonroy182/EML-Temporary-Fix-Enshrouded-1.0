# EML-Temporary-Fix-Enshrouded-1.0
A Fork with patched code for EML Enshrouded Game by Brabb3l (github.com/Brabb3l/kfc-parser) version 1.0 of the game.

## M182 native layer and Creature Spawner (work in progress)

`native-layer/src` holds the Rust sources of the native layer (placed in `crates/mod-loader-runtime/src/native` of the EML fork): PvP faction hooks and the creature spawner (hook of the entity-create function, live player position, hotkey spawn by enemy name). `native-layer/m182_native.example.json` is the configuration file. `spawner-mods/m182_spawn_variants` is the Lua mod that exports the enemy list and adds stronger enemy variants.

## Downloads

Every push to `spawner-panel`, `eml-fork`, `nexus-package` or `spawner-mods` is built by GitHub Actions and published in the [latest build release](https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0/releases/tag/admin-pannel-latest):

- `M182_AdminPannel.exe` (also as `.zip`): the M182 Admin Pannel. Put it next to `enshrouded.exe` and open it while the game runs.
- `M182_PvP_Nexus.zip`: the package published on Nexus Mods (`dbghelp.dll`, `m182_native.json`, `eml.json`, the `m182_spawn_variants` mod and a README).
- `SHA256.txt`: checksums.

`eml-fork/` holds the full source of the modified EML (GPL-3.0) that produces `dbghelp.dll`; `native-layer/` and `spawner-panel/` hold the M182 sources. Nexus description: `docs/Nexus_description.txt`.
