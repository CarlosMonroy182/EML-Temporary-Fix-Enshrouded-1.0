# M182 Market (separate mod, not part of the Admin Panel)

Status: phase 1 built and tested in-game by the owner; moved out of the Admin Panel on Oct 23.

What exists
- Native chat commands in `native-layer/src/market.rs` (active only when `m182_market.json` sits next to the game/server):
  `!market balance | list [page] | buy <n|name> [qty] | pay <player> <amount>` (Spanish: `!mercado saldo | lista | comprar | pagar`).
- Wallets are virtual (`m182_wallets.json`, server side). Bought items are dropped next to the buyer
  (generic pickup template `c15a5c9d-4956-49ff-9d72-9f522229cf1c`, same path as spawner deliveries).
- Server-published state for a panel: `m182_market_live.json` (balances + latest events) and the admin chat query
  `!m182 <key> wallets <seq>` (answer scanned from chat memory, same channel as teams/roster).
- `panel/market.rs`: the panel tab that was removed (catalog browser with name/category/rarity filters, offers editor,
  balances, grants, recent activity). Re-add it as the UI of a standalone market panel.

Findings
- Grants (coins from the admin) are applied by the server tick as soon as the file arrives.
- Real in-game coins need inventory read/remove access (not reverse engineered). Without it the currency stays virtual.
- Not built: NPC vendor, kill rewards (needs enemy-death detection), selling items back, player chest stalls, Discord notices.
- Client agreed phases; auctions were rejected (too much work).
