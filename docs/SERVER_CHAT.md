# M182 chat commands (dedicated server)

Set `"chat_key"` in `m182_native.json` (at least 6 characters, no spaces) and restart the server.
An admin can then type in the game chat:

    /m182 <key> ping
    /m182 <key> pvp on|off
    /m182 <key> set <file> <json>
    /m182 <key> put <file> <first part>     (long content: put, add..., end)
    /m182 <key> add <file> <next part>
    /m182 <key> end <file>

Messages that start with `/m182` (or `!m182`) are hidden from the chat. Results are written to `m182_spawner.log`.
Files: m182_spawn.request, m182_command.request, m182_powers.json, m182_zones.json, m182_teams.json, m182_spawners.json.
Eight wrong keys in a minute lock the channel for that minute.
