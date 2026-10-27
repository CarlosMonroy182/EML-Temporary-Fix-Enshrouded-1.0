M182 INFERNO MAGIC - Enshrouded 1.0 (Windows)

Everything needed is in this archive. Nothing else has to be downloaded.

INSTALL - single player or hosting a world
1. Close Enshrouded.
2. Open the game folder (Steam > Enshrouded > Manage > Browse local files), the one with enshrouded.exe.
3. Copy these files into that folder:
     dbghelp.dll
     m182_native.json
     m182_pillar.json
     mods   (the whole folder; if you already have a "mods" folder, copy the folder "m182_inferno_beam" inside it)
     eml.json   (skip it if you already have one; then make sure "use_export_flag" is true in yours)
4. Start the game and cast the Eternal Pillar Of Flames or the Eternal Veil Beam.

INSTALL - dedicated server
1. Stop the server.
2. With FTP, go to the folder where enshrouded_server.exe is.
3. Upload dbghelp.dll, m182_native.json, m182_pillar.json, eml.json and the folder mods (or only mods/m182_inferno_beam if the server already has a mods folder) there, replacing what is there.
4. Start the server. Players join with the normal game. The big pillar and the wide damage area of the Veil Beam work for everyone; to also see the thick Veil Beam, a player copies the folder mods/m182_inferno_beam, together with this dbghelp.dll, into his own game folder.

CONFIG (m182_pillar.json) - read when the game or server starts
  "mode": "on"            "on" enables the mod, "off" disables it
  "radius_m": 15.0        radius of the damaging fire, in meters
  "duration_s": 15.0      how long the fire lasts, in seconds
  "set_duration": true    false keeps the original duration
  "admin_only": false     false = every player gets the big pillar; true = only players with the kick/ban permission (on a single-player world, the host)
  "visual": true          adds the ring of big flames around the pillar, false = only the damage area grows
  "visual_scale": 4.0     size of each flame of the ring
  "visual_ring": 8        number of flames in the circle (0 = only the centre one)

VEIL BEAM (mods/m182_inferno_beam/src/mod.lua) - values at the top of the file, read when the game or server starts
  HIT_RADIUS = 1.5        radius of the damaging beam, in meters (original 0.2)
  VISUAL_RADIUS = 1.5     radius of the drawn beam, in meters (original 0.1)
  LIGHT_RADIUS = 12.0     radius of the light of the beam (original 3.0)
  Delete the folder m182_inferno_beam to get the original beam back.

If you already use other mods that install their own dbghelp.dll, keep only one dbghelp.dll in the folder. This one loads the mods of the "mods" folder like EML does.

UNINSTALL
Delete dbghelp.dll, m182_native.json, m182_pillar.json, the folder m182_inferno_beam (and eml.json if you did not have it before).
Use "Verify integrity of game files" in Steam if you also used other mods.

Made by M182. Based on EML by Brabb3l (GPL-3.0), distributed under GPL-3.0, see LICENSE.
Source code: https://github.com/CarlosMonroy182/EML-Temporary-Fix-Enshrouded-1.0
Buy me a coffee: https://www.paypal.me/MoradoGames
