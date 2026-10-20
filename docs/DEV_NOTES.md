# M182 Enshrouded 1.0 mods - development notes

Repo: CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0, branch `m182-native-layer`. Only this repo is used; never the upstream Brabb3l repo.

## Layout
- `eml-fork/` - fork of EML (dbghelp.dll proxy mod loader) patched for 1.0. `crates/mod-loader-runtime/src/native/` is the M182 native layer (Rust, built into `dbghelp.dll`). `native-layer/src/` is an identical copy kept for reading and tests; keep both in sync.
- `spawner-panel/` - egui admin panel, built as `M182_Panel.dll` (cdylib, opened with Insert in game). Source language is Spanish. `tools/localize.py` + `tools/i18n_en.py` rewrite string literals to English at build time.
- `spawner-mods/` - Lua mods: `m182_spawn_variants` (creature/item lists export) and `zz_m182_item_stats` (applies stats, levels, balance from `m182_data.lua`).
- `release-package/{Cliente,Servidor}` - json files shipped in the zips. `release-package/text/{es,en}` - LEEME/README texts.
- `dev-tools/native-tests` - Linux test harness for the native layer.
- `.github/workflows/build-admin-panel.yml` - builds panel (ES and EN), native `dbghelp.dll`, packages and publishes release `admin-pannel-latest` with `M182_Cliente_ES.zip`, `M182_Servidor_ES.zip`, `M182_Client_EN.zip`, `M182_Server_EN.zip`, `M182_PvP_Nexus.zip`.

## Panel <-> game protocol
- Panel and DLL talk through files in the game folder (`m182_powers.json`, `m182_zones.json`, `m182_teams.json`, `m182_spawners.json`, `m182_player.json`, `m182_online.json`) and logs `m182_panel.log`, `m182_spawner.log`.
- Against a dedicated server the panel types `!m182 <key> ...` into the game chat; the server hides these messages. Key: `remote_key` (client `m182_native.json`) = `chat_key` (server).
- Zones, teams, spawns, PvP live on the server and apply in seconds. Item stats, levels, balance and free skills are written to `mods/zz_m182_item_stats/src/m182_data.lua` and applied only when the server starts.
- Spawner states are Spanish protocol strings (`vivo`, `creando`, `esperando para reaparecer`, `esperando el mundo`): never translate them.
- Chat client id -> entity `(client & 0x3f)+1`.
- Visitor restriction (protected bases): the server permission getter is hooked (signature `PLAYER_PERMISSIONS`); visitors lose bits 2|4|8|0x10. Log lines: `permissions hooked`, `is a visitor now`.

## Removed on purpose
Unique weapon copies (custom item pools) and panel teleport / map travel were removed. A distinct weapon is made by changing item level. Old files `m182_custom.txt`, `m182_custom_items.csv`, `m182_custom.skipped`, `m182_custom.running` are no longer used.

## Tests
- Native: `cd dev-tools/native-tests && python3 sync.py && cd linuxtest && cargo test --offline -- --test-threads=1` (sync.py generates `linuxtest/src/native` from `native-layer/src`; run it after every native change).
- Panel: `cd spawner-panel && cargo test --offline --features linux-preview --lib -- --test-threads=1`.
- English check: `python3 spawner-panel/tools/localize.py spawner-panel/src <out>/src`, copy Cargo.toml, Cargo.lock and assets next to it, run the same panel tests there. The script fails if a Spanish literal is left untranslated or a table entry is unused. New UI text needs an entry in `tools/i18n_en.py`.
- `shipped_baseline_mod_matches_the_generator` checks `spawner-mods/zz_m182_item_stats/src/mod.lua` equals the generator output; regenerate with `M182_WRITE_BASELINE=<dir>`.

## Release
1. Run the tests above. 2. Commit (trailers `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and the session line) and `git push origin HEAD:m182-native-layer`. 3. Wait for the workflow, then download the zips from `https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0/releases/download/admin-pannel-latest/<name>.zip`. 4. Deliver the four zips. The zips do not contain the user's item data (`m182_data.lua`, `m182_item_stats.json`).

## Not confirmed in the real game
Chest/building block for visitors, safe zone entry cue, solo-player roster by Steam name. Ask for `m182_spawner.log` (server) and the EML log when something fails.

## Mode detection (hosting vs connected)
- `m182_mode.json` {"hosting":bool} decides whether the panel applies commands locally or types them into the chat. A game that first loaded its own world and then joined a server kept the old server manager (the client manager address is reused, so the "client manager changed" check never fired) and stayed on hosting=true: commands were applied locally, nothing reached the chat. `check_hosted_world()` (1 Hz) now forgets the hosted world once its player cannot be read for 10 checks while a client manager exists. Symptom in logs: "spawn requested from the panel" with no "chat:" lines on the server and no "mode: connected to a remote server" on the client.
- Spawner changes travel as `m182_spawner_op.json` ({"add":row}/{"rm":id}/{"set":[rows]}, short keys i,t,p,l,c,h,d,r,w) merged by the server into m182_spawners.json.

