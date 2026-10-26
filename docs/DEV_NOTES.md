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

## Chat typing leaks (keys reaching the game as hotkeys)
- The panel types lines with real scan codes (the game ignores Unicode key events). If the Enter that opens the chat is not registered (game running slowly: sampled per frame), the hex text is read as hotkeys: `enshrouded.log` shows bursts of `[GameUi] Map Menu opened / Entered Journal / opened Backpack` at the time of each send. Healthy sends leave one `[input] closeVirtualKeyboard` + `toggleHudScreen to InGame` per line, ~0.5 s apart; the first one comes ~1.0 s after `Window became active`, ~1.3 s when the game lags.
- Mitigation: the opening key is held (`typing_hold_ms`, default 120 ms) and the chat gets 300 ms to open; each line is confirmed by a new `closeVirtualKeyboard` in `enshrouded.log` (`LogAcks`). Once one confirmation was seen in the session, a missing one stops the job (no retry, `[stop]` marker) instead of typing on. If no confirmation ever shows, the check switches itself off.
- Not solved at the root: there is no way to see that the chat is open before typing. A real fix would be calling the game's chat enqueue from the client's native layer (needs the context pointer of a genuine call).


## Own character on a remote server (client side)

A game connected to a dedicated server sees every player's character. The creation hook used to take the first player entity it saw (usually the server's oldest player) as "me", so `m182_player.json`, "Create where I am" for zones/spawners/places and the distance notes used another player's position. Evidence: the client's logged position equalled the server's position of the first player to two decimals at the same moment.

Fix: entity numbers are shared between server and client. The panel reads the server's player list (`m182_online.json`), finds this account by Steam id (or by unique name) and writes `m182_self.json` (`{"entity":N}`). The native layer (`apply_own_character`) switches `CLIENT_PLAYER_ID` to that entity once the client world has its record, ignores the file if it is older than the process, and `after()` stops overwriting it until the client manager changes. Log line: `client: this game's character is entity N`.

### Position comes from the server

The server's player-list answer now ends with ` ^<entity>@x,y,z`: the entity of whoever asked (chat client `(n & 0x3f) + 1`) and its position as the server sees it. The client writes it to `m182_online.json` as `"me"`. "Create where I am" (zones) and "Save my position" (places) ask for a fresh list and use that position (`App::ask_here`), falling back to the game's own position only when the server did not send one. The same `me.n` is what the panel writes to `m182_self.json`. The panel log has a `position check:` line comparing both positions.

## Every dbghelp.dll build must keep
- the EML 0.2.0 loader fixes (commit e36ae2f: no crash/errors on close; markers "KFCWriter is no longer available", "Failed to apply resource")
- the M182 VFX module.
The CI step "Guard - dbghelp.dll must keep EML 0.2.0 loader fixes and the M182 VFX" fails the build if a marker is missing.

## Market (native `market.rs` + panel `market.rs`)
- Balances (`m182_wallets.json`, server-written) and catalog/grants (`m182_market.json`, panel-written, forwarded to the server like teams). Grants have unique ids; the server applies each once.
- Chat: `!market|!mercado balance|list|buy|pay`. A purchase deducts coins first, then queues pickups through `spawner::queue_delivery` (same path as the panel's item delivery, one pickup per copy, next to the buyer). Optional `zone` restricts buying to a named zone (positions come from `groups_tick` via `market::note_position`).
- Not done yet: kill rewards, selling items back, player stalls (needs reading/removing chest and inventory contents).
- Market only (no admin tools): when `m182_market.json` exists at startup the DLL hooks the chat and the create function by itself, even with no `m182_native.json`, `chat_key` empty and `spawner` false. Only `!market`/`!mercado` are handled then; a market thread (`market_loop`) runs deliveries and grants, `players_loop` tracks players. Hotkeys stay off. Package: `nexus-package/market-test` -> `M182_Market_Test_{Client,Server}_EN.zip`. The `zone` rule needs the Admin Panel zones, so it stays empty here.
