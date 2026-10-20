local CHARGE_NOTIFIER = "ba663a7a-ba30-4ed3-8ae8-c6ec1d440304"
local COLOR_TYPE = 1392743968
local TAG = "[m182-vfx]"

local PRESETS = {
    original = "8822f231-cfdc-493e-94da-dcb68762ef11",
    rayos = "df4962ec-1eaa-4c23-a8c1-0e5ac994f275",
    fuego = "f850bc76-0efb-4c80-a77e-da0fd5dffd60",
    hielo = "8617bf03-e195-445e-9409-c89ff6a8a5f9",
    niebla = "8685a499-da08-4290-8f1c-686e34c85699",
    luz = "86962dd8-fc0c-425a-a9d3-2495fd7029a4",
    acido = "a4898fc5-e1a3-475c-b7dc-688e35401f07",
    hueso = "fc09f4cf-2b3f-4ef4-b257-1cd1cdd4239f",
    llama_arcana = "334d5410-faa3-4152-927b-98a64c2c992f",
    curacion = "a3d8af6b-d34f-4b1a-b633-a0a02a3b3c30",
    drenaje = "87c8d367-7684-4d8e-9a78-a25a59cc269e",
    sangre = "df1647ca-ea1f-4d6e-bb7f-2c8bf27a7381",
    sol = "db2ea485-9605-49a8-90e9-c84c5d881b5c",
    aura_fuego = "2232ae40-60be-4e08-b653-f13ac50522a0",
    escudo = "a7935575-4f91-4664-9ca7-81b3f353352e",
    implosion_rayos = "05cb2c93-b7fe-4f82-bfac-2196e99b8307",
    implosion_fuego = "ae3ec4fb-a3fb-4e97-97be-459e1d7f51ed",
    implosion_hielo = "05a66c81-dda1-49fa-8db7-6cbfe40b16f3",
    implosion_niebla = "c56bb712-a73d-460d-895e-1a14cffe9dcf",
    implosion_hueso = "d0316cdf-d9f6-46dd-badf-4d3a787a2607",
}

local ALIASES = {
    lightning = "rayos", rayo = "rayos", fire = "fuego", ice = "hielo", fog = "niebla", shroud = "niebla",
    light = "luz", acid = "acido", bone = "hueso", arcane = "llama_arcana", heal = "curacion",
    drain = "drenaje", blood = "sangre", sun = "sol", fire_aura = "aura_fuego", shield = "escudo",
}

local NAMED_COLORS = {
    rojo = "FF1A1A", red = "FF1A1A", naranja = "FF7A00", orange = "FF7A00", amarillo = "FFE600", yellow = "FFE600",
    dorado = "FFB23E", gold = "FFB23E", verde = "1AFF3C", green = "1AFF3C", cian = "00F0FF", cyan = "00F0FF",
    azul = "1A5CFF", blue = "1A5CFF", morado = "9B30FF", purple = "9B30FF", rosa = "FF5CC8", pink = "FF5CC8",
    magenta = "FF00FF", blanco = "FFFFFF", white = "FFFFFF",
}

local function try(f)
    local ok, r = pcall(f)
    if ok then return r end
    return nil
end

local function hash_of(v)
    if v == nil then return nil end
    if type(v) == "number" then return v end
    local inner = try(function() return v.value end)
    if type(inner) == "number" then return inner end
    return tonumber(tostring(v))
end

local function unsigned(n)
    n = math.floor(n)
    if n < 0 then n = n + 4294967296 end
    return n
end

local function parse_color(text)
    if type(text) ~= "string" then return nil end
    local s = string.lower((text:gsub("%s", "")))
    if s == "" then return nil end
    s = NAMED_COLORS[s] or s
    s = s:gsub("^#", "")
    if not s:match("^%x%x%x%x%x%x$") then return false end
    local r = tonumber(s:sub(1, 2), 16)
    local g = tonumber(s:sub(3, 4), 16)
    local b = tonumber(s:sub(5, 6), 16)
    return 0xFF000000 + b * 65536 + g * 256 + r
end

local loaded, CONFIG = pcall(require, "m182_vfx_config")
if not loaded or type(CONFIG) ~= "table" then
    print(TAG .. " config not readable, using defaults: " .. tostring(CONFIG))
    CONFIG = {}
end

local key = string.lower(tostring(CONFIG.efecto or "rayos"))
key = ALIASES[key] or key
local target = PRESETS[key]
if target == nil then
    local names = {}
    for name in pairs(PRESETS) do names[#names + 1] = name end
    table.sort(names)
    print(TAG .. " unknown effect '" .. key .. "', available: " .. table.concat(names, ", "))
    return
end

local colors = {}
if type(CONFIG.colores) == "table" then
    for _, c in ipairs(CONFIG.colores) do
        local v = parse_color(c)
        if v then colors[#colors + 1] = v
        elseif v == false then print(TAG .. " invalid color '" .. tostring(c) .. "' ignored") end
    end
end
if #colors == 0 then
    local v = parse_color(CONFIG.color)
    if v then colors[1] = v
    elseif v == false then print(TAG .. " invalid color '" .. tostring(CONFIG.color) .. "' ignored") end
end

local notifier = try(function() return game.assets.get_resource(CHARGE_NOTIFIER, "keen::NotifierResource") end)
if notifier == nil then
    print(TAG .. " charge notifier not found, nothing changed")
    return
end
local vfx = try(function() return game.assets.get_resource(target, "keen::VfxResource") end)
if vfx == nil then
    print(TAG .. " effect " .. key .. " (" .. target .. ") not found, nothing changed")
    return
end

local nd = notifier.data
local vd = vfx.data
local source = vd.defaultParameterData.data
local params = {}
for i = 1, 16 do params[i] = unsigned(hash_of(source[i]) or 0) end

local slots = 0
for _, p in ipairs(vd.parameters) do
    if hash_of(p.typeId) == COLOR_TYPE then
        local index = math.floor((hash_of(p.dataRange.start) or 0) / 4) + 1
        if index >= 1 and index <= 16 then
            slots = slots + 1
            if #colors > 0 then
                params[index] = colors[((slots - 1) % #colors) + 1]
            end
        end
    end
end

local ok, err = pcall(function()
    nd.defaultVfx = target
    local block = nd.defaultVfxInitParameters.data
    for i = 1, 16 do block[i] = params[i] end
end)
if not ok then
    print(TAG .. " could not write the charge aura: " .. tostring(err))
    return
end

local written = try(function() return nd.defaultVfx end)
print(string.format("%s charge aura = %s (%s) written=%s color slots=%d colors applied=%s",
    TAG, key, target, tostring(written == target), slots, tostring(#colors > 0 and slots > 0)))
if #colors > 0 and slots == 0 then
    print(TAG .. " effect " .. key .. " has no color option, its own colors are used")
end
