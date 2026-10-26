M182 INFERNO MAGIC - Enshrouded 1.0 (Windows)

Everything needed is in this archive. Nothing else has to be downloaded.

INSTALL - single player or hosting a world
1. Close Enshrouded.
2. Open the game folder (Steam > Enshrouded > Manage > Browse local files), the one with enshrouded.exe.
3. Copy these files into that folder:
     dbghelp.dll
     m182_native.json
     m182_pillar.json
     eml.json   (skip it if you already have one; then make sure "use_export_flag" is true in yours)
4. Start the game and cast the Eternal Pillar Of Flames.

INSTALL - dedicated server
1. Stop the server.
2. With FTP, go to the folder where enshrouded_server.exe is.
3. Upload dbghelp.dll, m182_native.json, m182_pillar.json and eml.json there, replacing what is there.
4. Start the server. Players join with the normal game, they do not install anything.

CONFIG (m182_pillar.json) - read when the game or server starts
  "mode": "on"            "on" enables the mod, "off" disables it
  "radius_m": 15.0        radius of the damaging fire, in meters
  "duration_s": 15.0      how long the fire lasts, in seconds
  "set_duration": true    false keeps the original duration
  "admin_only": true      true = only players with the kick/ban permission get the big pillar (on a single-player world, the host); false = every player
  "visual": true          adds the ring of big flames around the pillar, false = only the damage area grows
  "visual_scale": 4.0     size of each flame of the ring
  "visual_ring": 8        number of flames in the circle (0 = only the centre one)

If you already use other mods that install their own dbghelp.dll, keep only one dbghelp.dll in the folder. This one loads the mods of the "mods" folder like EML does.

UNINSTALL
Delete dbghelp.dll, m182_native.json, m182_pillar.json (and eml.json if you did not have it before).
Use "Verify integrity of game files" in Steam if you also used other mods.

Made by M182. Based on EML by Brabb3l (GPL-3.0), distributed under GPL-3.0, see LICENSE.
Source code: https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0
Buy me a coffee: https://www.paypal.me/MoradoGames
