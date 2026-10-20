-- M182 VFX - configuracion
-- Cambia los valores, guarda el archivo y abre el juego. El cambio se aplica al iniciar.
--
-- efecto: aura que aparece al cargar un ataque (baston apuntando, espadas, dagas, etc.)
--   "original"         aura original del juego (acepta color)
--   "rayos"            rayos electricos
--   "fuego"            fuego
--   "hielo"            hielo / ventisca
--   "niebla"           niebla del Shroud
--   "luz"              luz sagrada
--   "acido"            acido
--   "hueso"            magia de hueso (acepta color)
--   "llama_arcana"     llama arcana (acepta colores)
--   "curacion"         curacion (acepta colores)
--   "drenaje"          drenaje de mana (acepta colores)
--   "sangre"           magia de sangre (acepta colores)
--   "sol"              aura solar (acepta color)
--   "aura_fuego"       aura de fuego (acepta color)
--   "escudo"           escudo magico (acepta colores)
--   "implosion_rayos"  implosion de rayos (explosion al empezar a cargar)
--   "implosion_fuego"  implosion de fuego
--   "implosion_hielo"  implosion de hielo
--   "implosion_niebla" implosion de niebla
--   "implosion_hueso"  implosion de hueso
--
-- color: "" deja el color del efecto. Si no, "#RRGGBB" (ej. "#9B30FF") o un nombre:
--   rojo, naranja, amarillo, dorado, verde, cian, azul, morado, rosa, magenta, blanco
-- colores: para efectos con varios colores, una lista que reemplaza a "color",
--   ej. colores = { "#FF0000", "#FFFFFF", "#9B30FF" }

return {
    efecto = "rayos",
    color = "",
    colores = {},
}
