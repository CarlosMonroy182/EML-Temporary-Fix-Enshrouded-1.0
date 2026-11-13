# M182 Enshrouded 1.0 mods - development notes

Repo: CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0, branch `m182-native-layer`. Only this repo is used; never the upstream Brabb3l repo.

## Layout
- `eml-fork/` - fork of EML (dbghelp.dll proxy mod loader) patched for 1.0. `crates/mod-loader-runtime/src/native/` is the M182 native layer (Rust, built into `dbghelp.dll`). `native-layer/src/` is an identical copy kept for reading and tests; keep both in sync.
- `spawner-panel/` - egui admin panel, built as `M182_Panel.dll` (cdylib, opened with Insert in game). Source language is Spanish. `tools/localize.py` + `tools/i18n_en.py` rewrite string literals to English at build time.
- `spawner-mods/` - Lua mods: `m182_spawn_variants` (creature/item lists export) and `zz_m182_item_stats` (applies stats, levels, balance from `m182_data.lua`).
- `release-package/{Cliente,Servidor}` - json files shipped in the zips. `release-package/text/{es,en}` - LEEME/README texts.
- `dev-tools/native-tests` - Linux test harness for the native layer.
- `.github/workflows/build-admin-panel.yml` - builds panel (ES and EN), native `dbghelp.dll`, packages and publishes release `admin-pannel-latest` with `M182_Cliente_ES.zip`, `M182_Servidor_ES.zip`, `M182_Client_EN.zip`, `M182_Server_EN.zip`, `M182_PvP_Nexus.zip`. Also `M182_Nexus_Client.zip`, `M182_Nexus_Server.zip` (public, no key).

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

## Caps Lock on the panel user's PC (GPortal client report, Nov 3, confirmed fixed in the game)
- Symptom: after some minutes every panel line is rejected with `chat: client N used a wrong key`, then `ignored (too many wrong keys)`; nothing works (items, teams, chests), though zones already applied keep working. Keys in both m182_native.json files were identical and the server config was not reloaded in between.
- Cause: the panel types real scan codes with Shift taken from `VkKeyScanW`; with Caps Lock on, every letter arrives with its case flipped. The prefix (`!M182`) and command names are compared ignoring case, the key is not, so only the key fails.
- Fix: the panel inverts Shift for letters when Caps Lock is on (`caps_lock_on()` in os_windows.rs); the server also accepts a line whose key matches with all letters flipped and flips the whole line back (log suffix `[typed with Caps Lock on: letters flipped back]`). The wrong-key log line now gives the received length and matching prefix length without revealing the key.

## Panel thinks the server has a file it rejected (Nov 3, confirmed)
- `m182_sent.json` records a file as delivered once the game confirms the chat lines left, even if the server rejected them (wrong key, lockout). The panel then skips that file ("skipped ...: the server already has this exact content") and the server keeps the old copy.
- Symptom seen: zone saved before its team existed (`0 owned bases`, `visitors=[]` for everyone) although the panel shows the zone with an owner team and "Base protegida".
- Fix for the user: "Reenviar todo" / "Resend all" (forgets the record and sends powers, zones, teams, spawners, item data). Check `zones: N active (.., 1 owned bases)` and `is a visitor now` in the server log.
- Not changed in code: zones/teams are not resent automatically on join (the server is the source of truth for teams changed by chat while the admin is away, and the same key is used on more than one server).

## M182 Building (mods/m182_diag_build): doors and stations (Nov 3, confirmed)
- Doors and crafting stations get an `InteractionLock` copied from `WaterDrain_ground_base_PLAYER`. That donor requires `CanEditBase`, so server groups with `canEditBase: false` (GPortal client group "Miembro") lost doors and stations everywhere, not only in protected bases.
- Now every inserted lock is retargeted to `requiredPermission = {"CanAccessInventories"}` (config key `door_lock_permission`), the same permission chests use. Visitors of a protected base lose it through the permission hook (bits 2|4|8|0x10), team members and other groups keep it.
- PlayerPermissions bits (from the game's types.json): CanKickBan 1, CanAccessInventories 2, CanEditBase 4, CanEditWorld 8, CanExtendBase 0x10, CanReceiveEXP 0x20.
- Doors covered: the 23 `*Door*Crafted*` templates plus every template placed from the building menu whose name has door/gate/portcullis (not frames). World doors are not touched.
- EML log lines: `door and station lock needs CanAccessInventories (lock source had {CanEditBase}, now {CanEditBase}; N locks changed ...)`, `doors added from the building menu: ...`, `door pieces without an open/close interaction (unchanged): ...`. If the source ever reads "now {CanAccessInventories}", the lock is shared and the water drain changed too.
- Delivery for customers that already have the mod: only `mods/m182_diag_build/src/mod.lua` (never the config file), then restart the server.

## GPortal hosting
- GPortal runs the server natively on Windows (`D:\home\sid_...\enshrouded`), unlike Dathost (Wine). Same files and install steps; server groups and passwords live in `enshrouded_server.json` (`userGroups` with canKickBan/canAccessInventories/canEditWorld/canEditBase/canExtendBase).

## Character guard (server, Midnight Rain lock) - findings Nov 5
- Stage 1 (`guard.rs`, `m182_guard.json` {"mode":"observe"}) works on the current server build: `SAVE_DESERIALIZE` is unique in enshrouded_server.exe (absent from the client exe); upload object still `+0x110` blob ptr, `+0x118` length.
- The uploaded blob is `0x01` + a zstd frame. Decompressed it starts `00 <u32> "S2BH" 02 00 00 00 ...`. The `character` id printed by stage 1 (bytes 0xC..0x1C of the compressed blob) is not a real id.
- After five 1.0f floats (around offset 0x4D of the decompressed data): u32 level, u32 XP, u32 unknown (stays equal between sessions). Seen: level 22 with XP 3946 -> 4496 -> 4741 over three joins of the same character.
- Inventory items appear as plain u32 item ids (same ids as `vfx_items.csv`); spells are 11 bytes apart, weapons 24, so records are variable length.

## Sending chat from code (chat_inject, build 1267041)

The panel no longer has to type into the game. Findings (enshrouded.exe RVAs):

- `0xeb1220` is the game's "queue a chat line to send": `(out, queue, kind, &{ptr,len<=0x400}, tag1, tag2)`. It is what the
  typed chat box ends up calling (through the command dispatcher at `0x85c81a`). It appends to the list at `queue+0x208`.
  `CHAT_ENQUEUE` (`0xe6e1c0`) is the *receive* side (list at `queue+0x1c0`) of the same queue object.
- `0x871ef0` is the per-frame sender: it pops `queue+0x208` through `0xe86ce0`, encodes a `ChatMessageClientMessage`
  (`0x11f6420`) and releases the entry with `0xea79a0`. Its object holds the queue at `+0xf8`. Everything above runs on the
  game thread, as does the `filter_permissions` hook, so the call is made from that hook (`outbox::pump`).
- `outbox.rs`: `flush_seen` (lean hook on the per-frame sender, two stores) keeps the queue pointer and the time of the last
  frame; lines are only queued when that is < 1.5 s old. `m182_chat_out.ready` holds `1` while that is true. The panel's
  `InjectTypist` writes `m182_chat_out.txt` when ready and falls back to the keyboard typist otherwise.
- Signatures: `CHAT_SEND` (48 bytes, unique) and `WATCH_FLUSH`. Direct call, no hook on the send function itself.

## Shield / god mode cost

A failed record lookup walks all 640 component slots. On a game client the server owns the health record, so the
shield did that every 4 ms. `spawner.rs` now remembers misses per (manager, entity, record) with a wait that doubles up to 8 s
(`MISSING`), scans through the cached archetype shapes (`component_quick`), and when nothing can be protected the power loop
polls every 50 ms instead of every 4 ms (`SHIELD_IDLE`).

## Panel answers (player list, teams) without chat noise

- The server answers `[M182#n] ...` / `[M182T#n] ...` by rewriting the asker's message. `patch.rs::send_to_asker_only` now queues it
  itself and then marks the queue entry as delivered for every peer but the asker (`entry+0x20` bit mask, per-peer byte counters at
  `queue+0x14+12*peer`, peer = `client & 0x3f`), so other players never receive it.
- A game connected to a server (`chat_key` empty, `chat_inject` on) swallows those lines in `hook_chat` (`take_server_reply`) and writes
  `m182_online.json` / `m182_teams_live.json` directly. The old memory scan (`scan_request`) is skipped then; it stays as the fallback.
- `roster_line` lists the player table only (connected players); tracked entities are used only while the table is empty.
- `power_tick` logs `slow power tick: N ms (part ...)` when a tick takes >= 250 ms.

## Shared server state (several admins, one truth)

The server is the source of truth for zones, places, spawners, teams and raid templates. A panel on a clean install learns all of it
by asking, and stays current without polling.

- Topics (`native/shared.rs::Topic`): `zones places spawners teams templates` (shared) and `status` (on demand only). Each has a
  content-hash revision (FNV folded to 8 hex). The teams revision covers the panel definition and the in-game join/leave file
  (`m182_members.json`), so a leader inviting or kicking in the game changes it.
- Commands after the key check: `rev <seq>` returns `[M182R#seq] z= p= s= t= r=`; `pull <seq> <topic>` returns the topic as
  `[M182S#seq topic i/n hash] chunk` lines (envelope `{"topic","hash","data"}` -> deflate -> base64url, 600 chars per line).
  Both reach only the asker (`send_lines_to_asker`, queue-entry mask narrowing). The client hook swallows them and writes
  `m182_remote_rev.json` / `m182_remote_<topic>.json`.
- Writes are per-item ops through `m182_state_op.json`: `{"t":topic,"add":[],"up":[],"rm":[names]}`. `add` never replaces an existing
  different item (it is reported as taken), `up` upserts, `rm` is by name. Limits: 500 zones/places, 100 teams, 60 templates.
  Full-file writes of the shared files are still accepted (older panels) and count as a change.
- Teams are `panel definition + in-game overlay`; moved people get a `touched` counter assigned by the server so the later change wins;
  a person belongs to one team.
- Push: `note_change()` sets a flag when a state op, legacy write, spawner op or invite change happens; after the command is handled
  `tell_other_panels` sends the rev line to registered panels only (clients that proved the key in the last 12 min, identity still
  matching the table slot, never the author). No timer, no scan.
- Panel (`spawner-panel/src/shared.rs`): keeps the last server snapshot per topic, diffs local edits against it and sends only the
  differences. Nothing is sent before the first snapshot (this removed the path where a stale or wrong-profile team file was pushed:
  the "Admin MD" team of another profile). A 240 s safety rev check covers lost lines; `status` is pulled only while the spawner
  list is visible. In remote view the lists come from the server and local profile files are not written; leaving the server
  reloads the local files. "Subir mi perfil al servidor" uploads the active profile once (create-only).
- Tests: `dev-tools/native-tests/linuxtest/tests/shared_state.rs` (clean install learns everything, wrong key, split ops),
  `reply.rs` (multi-panel narrowing), `panel_protocol.rs`; panel: `cargo test --offline --lib -- --test-threads=1`.
- Not verifiable without the real game: delivery of pushes to several panels through the narrowed mask and long multi-line replies
  against the chat queue limits. The panel retries after 25 s (3 tries). Ask for `m182_spawner.log` (server) and the panel log.
- `power_tick` has 13 stage marks to find the cause of the multi-second "requests" stalls in the next logs.
- A game that hosts the world and also runs the panel (`inject_on` and the asker is peer 0) never receives its own answers through the chat
  (`is_remote_client()` is false there), so `patch.rs::accept_reply` reads those lines directly, for the asker's `rev`/`pull` answers and for
  pushes to the local panel. Other admins connected to that host still get theirs through the narrowed chat entry.
- Panel server strip: three fixed-height rows (status + "Reenviar todo", "Sincronizar" + sync message, upload hint) with truncated
  labels, so the layout does not move with the message length.
- `m182_mode.json` is rewritten until it succeeds (a failed rename used to leave the previous session's `false` for a hosting game, which made
  the panel treat its own hosted world as a remote server). The local panel also gets player-list and team answers directly (`accept_reply`).
- The panel logs every shared-state event (`shared: ... arrived / asked again / no answer`) in `m182_panel.log`; the header line is red when
  the server has not answered for 15 s or a topic ran out of tries.
- Compatibility: a panel whose server never answers `pull` (older `dbghelp.dll`) falls back to the classic behaviour after 3 tries
  (`shared::legacy()`: local lists, whole-file sends, live teams through the old scan) and probes with `rev` every 5 minutes; the header says in red
  that the server needs the new `dbghelp.dll`.
- `outbox::poll` (reads `m182_chat_out.txt`, writes `m182_chat_out.ready`) runs on its own `m182-outbox` thread. The client log showed
  `slow power tick ... (chat file 3-7 s)` there (a slow disk operation), which delayed spawn orders; the power loop no longer waits for it.

## Clients read the server's answers from memory until the chat hook proves itself

- `patch::replies_read_by_hook()` is true only after `take_server_reply` has really read an answer (`HOOK_TAKEN`). Before that, a client
  that is connected to a server keeps the old memory search for the player list and teams (`scan_request`), exactly as before the hook existed.
- Shared state (`rev` / `pull`): `outbox::pump` tells `spawner::expect_shared_reply(seq)` which request it just sent; `scan_shared_replies`
  (thread `m182-replies`) looks for `[M182R#seq` / `[M182S#seq` lines in memory every 400 ms for at most 14 s per request, feeds them to the
  same receivers as the hook, and stops as soon as the answer is complete. Idle clients run nothing. The log says how many answers were
  read, or that none appeared (server without this version).

## A dedicated server has no local panel

`patch::is_local_panel` (answers read in-process instead of queued to the chat) requires an empty chat key. A dedicated server always has
a chat key, and its first connected player (client 64, peer 0) is a remote panel; treating that player as local made the server consume its
own answers and nothing reached the client.

## Private answers fall back to the open chat

The per-peer narrowing of an answer (`narrow_last_entry_to`) was never confirmed to reach the asker on a real dedicated server: the server
logged "sent" and the client received nothing. `outbox` now remembers the requests (`rev`, `pull`, `players`, `teams`) it sends; if no
answer arrives within 8 s (`ANSWERED` is set by `accept_reply`, `store_roster`, `store_teams` and the memory search) it sends them again with
a trailing `open`, and every later request carries it. The server (`chat::take_open`) then answers the way it did before the narrowing:
to the whole chat for the player list and teams, unnarrowed lines for shared state. The server logs the first three narrowing attempts
(`chat: private answer for peer(s) ...`) with the active peers and the entry header, to find out why the private path fails.

## Gather call site on the dedicated server (Nov 6)
`hook_pay_costs` lets administrators build for free but the mining/gather call must reach the game untouched. The old check compared the caller with a client-only offset, so on `enshrouded_server.exe` mining paid nothing out. `is_gather_site` now recognises the call by the bytes around it (`GATHER_BEFORE` / `GATHER_AFTER`, unique in both executables) and caches the verdict per caller address.

## Cleanup of diagnostics (Nov 6)
Removed: `perf.rs` and the per-minute performance line, the slow-tick marks, the loot container reports and `m182-loot-report` thread, the pickup watch thread, the "chat seen", "player table", "entity definition looks like a player / shares x%" and narrowing diagnostics. Failure logs stay.

## Divine aura (Nov 6)
`aura` in the powers file (panel: Admin del jugador, checkbox "Aura divina"). `divine.rs` creates one `Prop_Ancient_Teleporter_Effect_On` (the glow of the tower teleport platforms, also used for the market statues; no model, no damage) under the administrator once he has stood still for 10 s (moved less than 1 m, checked every 500 ms only while the aura is on) and removes it as soon as he moves, switches the aura off or leaves. It does not follow the player: the prop cannot be glued to him and moving it every tick did not look right. Removal: writing Health=0 does NOT destroy this prop (no LifeTime, not destructible by health). The Lua mod `spawner-mods/m182_divine` adds a `keen::ecs::LifeTime` part (copied from template eee801de...) to the glow template with a huge lifetime and `enforceDestroy`, so the market statue lights are unchanged; the native layer creates the glow with `create_expiring` (600 s, enforceDestroy) and `spawner::expire_entity` writes lifeTime=1, endOfLife=1, enforceDestroy=1 into the entity's stride-24 LifeTime slot (found by the known lifetime values, index cached), so the game destroys it and replicates the removal. A glow is renewed after 540 s while the admin stays still. `divine::check_leaving` logs `ERROR` when a glow is still there 3 s after being expired or has no LifeTime (mod missing on the server/host). Creation happens in `spawner::before` (`divine::run_orders`). Spots of live glows are kept in `m182_divine_lights.json`; `divine::sweep` expires glows that load from the save near those spots and never touches other glows such as market statues. Nothing runs while no aura is on. `VFX_Teleport_Entity` and `VFX_Preview_Enemy_Mage_Teleport` were tried first: they gave only a dust column. The mod must be present on the dedicated server and on the PC that hosts.
- Nov 7 test on Midnight Rain (observe build): the server keeps serialized S2BH copies of each connected character in its own memory and rewrites them while the player plays (level 45/XP 11345 changed to 46/150 at the in-game level up). The next upload of that character matched the server copy, so the "state at exit" can be taken from the server's own serialization: next step is to hook the code that writes those copies (event driven, no polling) instead of the 10 s observe watch.
- Stable character id: the 16 bytes after `S2BH 02000000 0900 0500 000000 <flag>` (decompressed upload offset ~0x13). Same value for the same character across sessions and across servers (Aincrad Nov 5 and Midnight Rain Nov 7); different per slot. In memory the copies are preceded by a header whose u64 at -0x10 is the payload size (0x384a seen).

## Character guard, stage 2: server copies (Nov 7)
- `enshrouded_server.exe` save2 protocol (RVAs of the Nov 5 server build): `0x7144a0` receives a client message and finds the player object (`mgr + 0x56cf0 + (client & 0x3f) * 8`); `0x713f60` handles the character upload (checks the total size, an FNV-1a of the reassembled blob, then calls `0x714540` = `SAVE_DESERIALIZE`, logs "Failed to read client savegame" on error and stores the error in the player object at `+0x144`). Player object: `+8` number, `+0xc` save state (2 reading, 4 final save sent, 5 final save acknowledged), `+0x110` blob ptr / `+0x118` length.
- `0x733bf0` = `setPlayerSave(slot, &{data, size}, state, player)`: the server's own update of the copy it keeps while the player plays (it ignores new saves once state is 5). `0x73b2ac` logs "Server sent final client save" and sets state 4. Signature `SAVE_SET`, 16 bytes displaced.
- `guard.rs` (mode "observe" in `m182_guard.json`, nothing blocked, no timers): `probe` runs on every upload and compares the uploaded character (from its `S2BH` tag on) with the last copy the server kept; `on_set_save` keeps that copy per character id (`m182_guard/copy_<id>.bin`, written at most every 20 s and always at the final save, so it survives a restart). `m182_guard/guard.log` gets `join: ... identical to the server copy` or `differs ... first at byte N, K bytes differ ... around it upload X vs copy Y`, and `server copy: ...` when level/xp change or at the final save. The character id is read at `S2BH + 16`; the header hex is logged to confirm it.
- Next: read those logs from a real session to see how much a returning character differs from the server copy by itself, then decide the rule (identical copy, or progress not above the copy) and how to reject: make the upload read fail so the game takes its own "Failed to read client savegame" path.

### Guard, server copy hook diagnostics (Nov 7)
`0x733bf0` has no direct callers in the exe (it is reached indirectly), and the first observe build logged nothing from it. The hook now logs its first 12 calls ("save call N: slot player, state, span, blob size, first bytes") and accepts a span as {ptr,size}, {size,ptr} or {begin,end}. Per-player tick is `0x73ad80` (called from `0x73b426`): while playing it re-sends the server copy to the client every 5 s when the version counter at player+0x10 differs from the last sent one at +0xfc.

Result of the second observe test (Nov 7): `0x733bf0` never ran while playing (no "save call" lines). The server's authoritative copy is the final save it sends to the client when a player leaves: inside `0x73ad80`, chunks of 0x2bc bytes of the buffer at player+0x110 (length +0x118, hash +0x104, offset +0x108). The guard now hooks the first chunk (signature `FINAL_CHUNK`, mid-function stub that passes rbx and r13) and stores that blob as the server copy. Dumps show that a character that was not touched differs by about 5 bytes (counters) between two joins, so the comparison must be semantic or tolerate a few bytes.

Third observe test (Nov 7): the server's autosave to the client ("final save", every 30 s while playing) is the first ~1.5 KB of the character (header, level, XP, progress; unpacked 1530 bytes, the last 5 are a trailer) and it is a byte-exact prefix of what the client uploads at its next join. The remaining ~37 KB (items etc.) are not tracked by the server there. Bytes 512..548 are a clock block that changes with every save. Observe rules now logged on every join: progress (upload must equal the copy outside the clock block; "WOULD REJECT ... at byte N") and one character per account (`m182_guard/accounts.txt`, platform id from the player object via `roster::guess_id`). Nothing is blocked yet; enforcement needs more characters tested and the rejection path (make the game's own "Failed to read client savegame" fire).

Fourth observe test (Nov 8, several accounts and characters): the player object does not hold the platform id; the server's player table does (`table::entries()`, `client` == player number), so the account rule resolves it there (and again on the next save when the slot is not filled yet). The byte-exact prefix rule was too strict: every save stamps time counters at scattered offsets (52..54, 180..184, 211..225, ...), and the part of the character that changes when the inventory changes (records inserted/deleted around 168, 1427, 3871) is mostly outside the 0.2-1.5 KB the server copy covers. The progress rule is now semantic (level, XP, next); the inventory cannot be judged from this blob.

Inventory snapshot (Nov 8): the server's world holds each player's inventory (player entity `InventorySetup` 0x54 bytes -> page entities with `Inventory` 0x60 bytes of eight {item, count, instance} stacks; same layout the market uses). `spawner::inventory_items(entity)` reads it. The guard stores it per character (`m182_guard/inv_<id>.txt`, summed per item) with every save the server sends, and on the first save of a session compares it with the one stored when the character last left: "inventory: WOULD REJECT ... +item xN, -item xM". Observe only.

Account id (Nov 8): the Dathost server's player table has no Steam id; each slot keeps a 32-byte value at offset 72 that is identical in every session of the same account (`uid:<hex>`), used as the account key when no Steam id is found. Verify with a second account. Final version TODO (requested): drop every observation log line (join, final save, server copy, inventory snapshot, raw dumps, slot dump); keep only rejections, shown red.

## Guard final test phase (build 9)
- Observation removed. `m182_guard.json`: `mode` off|observe|enforce, `admins_exempt` (kick/ban permission in the slot), `one_character_per_account`, `new_character_max_level`, `verbose`. Allow list: `m182_guard_allow.txt` (account value or character id per line, read at every join).
- Rejection: `probe` (upload hook) zeroes the zstd magic of the uploaded blob in place, so the game's own deserialization fails and takes the "Failed to read client savegame" path. Not yet verified on a live server.
- Rules at upload: allow list, admin, one character per account (account = slot bytes 72..80 + 96..104; fail-open when missing or when two online players share it), then level/xp/next + plain item stacks (chain parsing of the first len(copy)-5 bytes) must equal one of the last 3 server copies; a new character may be capped by level.
- Server copies come from the FINAL_CHUNK hook (every ~30 s); stored in `m182_guard/held_<id>.bin`. Known risk: progress made after the last autosave before leaving may be refused (add to allow list or use observe).
- SAVE_SET hook, ECS inventory read and raw dumps were dropped.

## Reinforced objects per zone (Nov 13, not yet confirmed in the real game)
- Zone flag `shield` (panel: Zones, checkbox "Objetos reforzados", written only when on). `native/shield.rs`. What is built with the hammer inside the zone takes 40 times more to break; it can still be destroyed.
- Damage adds up. Kill decision `0x18b94c` (from `impact_hit_conclusion` `0x190670`): an entity with `Health` dies when it goes from above zero to zero. An entity without `Health` takes `Destructible.maxHealth` (u16), adds the hit's `healthChange` (negative), clamps to `[0, maxHealth]` and creates a `Health` component (`0xdc101091`, 0x28 bytes, ECS command type 3 at `0x18bf1b..0x18bf2f`) holding the remainder, with the maximum set to `maxHealth`; the next hits go through that `Health`. So the first design (one hit must be at least `maxHealth`) was wrong: 7-damage hits break a 25 or 40 point object in 4 to 6 hits and a 150 or 200 point one in 22 to 29.
- Mechanism: one write per object when it is created or loaded inside the zone, or when the flag changes. `Destructible.maxHealth` n becomes `n*40+39` (u16). For n above 1637 (2000, 4000, 10000 are the only such values among the 1366 placeable templates) the value becomes one of the fixed codes 65518, 65478, 65438. `Health` (when the entity has one: 58 templates, 40 of them also with `Destructible`) gets maximum `m*40+39` and current scaled in proportion (a full object stays full), written as one 32 byte span. A reinforced number never equals an original one (no template has a threshold equal to 39 mod 40), so a value is never multiplied twice and the flag switched off divides it back exactly; this also holds whether or not the game saves the component. `Health` created by a first hit holds the reinforced maximum and is divided back the same way. Health maxima that are not multiples of the pattern are taken as originals; unproven for `Health` of crafting stations whose maximum is read at run time and logged.
- Built = the entity carries `UsedItem` whose item is a `BuildTool` item of `export/m182_items.csv`. Item -> template: `ItemInfo.equipment.placedEntity` (1507 items, 1366 templates). Threshold values of those templates: 200 (170), 50 (135), 10 (126), 20 (107), 100 (102), 5 (93), 1000 (86), 1 (72), others fewer; 129 templates have no `Destructible`.
- Root cause of the failed Nov 12 test (`destructible None`): the game keeps two component tables. `*(manager)+0x88` is used for the components a create request carries (call at `0x61d868`: `UsedItem`, `CurrentTransform`), `*(manager)+0x30` for the components of the entity template (call at `0x61d686`: `Destructible`, `Health`). Both are the same hash map (`0x646ce0`), keyed by the template data's `qualifiedHash`, with the same u16 slot space. `component_index` now looks in both; the same offsets exist in the client create function `0x97b790`, so the hosted game is covered. If the table is not readable at some moment, the lookup is repeated every 64th request instead of giving up.
- When code runs (game thread, create hook only): a placement is switched right after the game created it; an entity created without `UsedItem` whose template can break (objects coming from the save) is looked at once, 300 ms later, on a following creation; after the zones change one pass goes over the loaded entities, 400 per creation. Nothing runs when no zone reinforces and nothing is released. `m182_shield_state.json` keeps the places that stopped reinforcing, so objects that load later in them are divided back.
- Trial: `m182_shield.json` `{"immune": true}` creates new placements in a reinforced zone with `CombatFlags.immune`.
- Log lines start with `object shield:`; budgets per reason (placed, reinforced, already, released, health, limited, nothing to break, other item, no position, unusual value, table, errors).

## Base guardian (trial, Nov 13, not yet tried in the real game)
- Panel: Zones tab, for a zone with an owning team: creature, level, "Colocar guardián aquí" (the admin stands inside the zone). Request `m182_spawn.request` with `"guardian": "<zone>"`. `native/guardian.rs`; nothing runs unless a guardian was placed (hooks return at once through one atomic).
- Hostility: `isEnemy(pointers)` and `isFriend(options)` receive the address of each entity's `Faction` byte. The guardian gets faction 11 (`PlayerAlly`), which the game's rules already make hostile to every creature faction; for a pair with a player-group faction `guardian::relation` decides by the guardian's faction address: friend when `groups::team_of(other address)` is the owning team (`friends()` is the one place for allied teams), enemy otherwise (other teams, players without a team, administrators without a team). The values variants carry no address and are only counted.
- Leash: `EnemySettings` (68 bytes, hash `0x67fe5508`; `maxTargetDistanceToHomeZone` at +60, `leashLength` at +64, f32) is written once to 0.9 of the zone radius; the home position is the place of creation. Runtime copies are `DynamicPathFinder`/`DynamicPathWalker.leashCenter/leashLength`. Unproven: whether the game reads these from the entity and what their unit is; the log shows the values before and after.
- Not saved: the create request carries `DoNotSave` (`0x41e678ea`), so the entity should not survive a restart. A guardian that cannot be adopted is retired (health to zero).
- Entities move in memory; the group tick (existing, 200 ms) calls `guardian::refresh`, which finds the faction address again, puts the faction back if it changed and forgets guardians that are gone.
- Log lines start with `guardian:`; at most 24 relation lines (who asked, faction, team, hostile or friend, distance of the guardian from the zone center).
