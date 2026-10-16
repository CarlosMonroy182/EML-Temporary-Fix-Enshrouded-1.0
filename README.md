# EML-Temporary-Fix-Enshrouded-1.0
A Fork with patched code for EML Enshrouded Game by Brabb3l (github.com/Brabb3l/kfc-parser) version 1.0 of the game.

## M182 native layer and Creature Spawner (work in progress)

`native-layer/src` holds the Rust sources of the native layer (placed in `crates/mod-loader-runtime/src/native` of the EML fork): PvP faction hooks and the creature spawner (hook of the entity-create function, live player position, hotkey spawn by enemy name). `native-layer/m182_native.example.json` is the configuration file. `spawner-mods/m182_spawn_variants` is the Lua mod that exports the enemy list and adds stronger enemy variants.
