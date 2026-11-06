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
