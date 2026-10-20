-- M182 VFX - configuracion
-- Cambia los valores, guarda el archivo y abre el juego. El cambio se aplica al iniciar.
-- Cambia el efecto que sale al apuntar y cargar un hechizo con el baston.
--
-- efecto: efecto para todos los hechizos. "original" deja el de cada hechizo.
--   rayos, rayos_mano, fuego, llama, hielo, escarcha, niebla, maldicion, niebla_muerte,
--   luz, acido, veneno, hueso, curacion, drenaje, ectoplasma, tentaculos, ardiendo,
--   aura_fuego, aura_agua, sangre, manos_brillantes, meteorico, aturdido
--   Aceptan color: llama, escarcha, veneno, hueso, curacion, drenaje, ardiendo,
--   aura_fuego, aura_agua, sangre, meteorico
--
-- color: "" deja el color del efecto. Si no, "#RRGGBB" (ej. "#9B30FF") o un nombre:
--   rojo, naranja, amarillo, dorado, verde, cian, azul, morado, rosa, magenta, blanco
--   Para efectos con varios colores puedes poner una lista: { "#FF0000", "#FFFFFF", "#9B30FF" }
--   Cada efecto tiene un solo color: si dos hechizos usan el mismo efecto, comparten color.
--
-- intensidad: de 1 a 10. Multiplica las particulas del efecto (1 = normal).
--
-- hechizos: efecto distinto para cada tipo de hechizo (opcional). Tipos:
--   fuego, llama, hielo, rayo, niebla, luz, acido, hueso, curacion, drenaje, agua
--   ej. hechizos = { fuego = "meteorico", hielo = { efecto = "escarcha", color = "cian" } }

return {
    efecto = "rayos",
    color = "",
    intensidad = 3,
    hechizos = {},
}
