M182 PvP + Admin Tools (in development) - host-only PvP, teams, safe zones and creature spawner for Enshrouded 1.0

Work in progress: the mod is updated regularly. The panel text is in Spanish (my original language); an English translation will come when the project is finished.

REQUIREMENTS
- Enshrouded 1.0 on Windows. Only the host needs the mod; friends join with the normal game (PC or PS5).

INSTALL (host only)
1. Close Enshrouded.
2. Copy the contents of this archive into the game folder, next to enshrouded.exe
   (Steam > Enshrouded > Manage > Browse local files):
     dbghelp.dll
     m182_native.json
     eml.json            (skip it if you already have one; then make sure "use_export_flag" is true in yours)
     mods\m182_spawn_variants\   (copy the whole folder into your "mods" folder)
3. Start the game once and load your world. The mod writes its lists to the "export" folder.
4. OPTIONAL - M182 Admin Pannel (M182_AdminPannel.exe): download it from the link below and put it
   in the game folder, next to enshrouded.exe. Open it while the game is running.
   https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0/releases/tag/admin-pannel-latest

CONFIG (m182_native.json) - reloaded while you play
  "enabled": true             master switch, false disables everything
  "pvp": true                 players can damage each other
  "pvp_friend_checks": true   needed for teams to protect their members
  "spawner": true             enables the creature spawner used by the panel

UNINSTALL
Delete dbghelp.dll, m182_native.json, the mods\m182_spawn_variants folder (and mods\zz_m182_item_stats if it exists).
Use "Verify integrity of game files" in Steam if you also used other EML mods.

NOTES
- If the game updates, the native part stays inactive until the mod is updated (it checks the game code first).
- Some antivirus programs flag DLL loaders like this one. The full source code is on GitHub.

Based on EML by Brabb3l (GPL-3.0). This package is distributed under GPL-3.0, see LICENSE.
Source code: https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0
Buy me a coffee: https://www.paypal.me/MoradoGames
