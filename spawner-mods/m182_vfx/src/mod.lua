local TAG = "[m182-vfx]"
local VFX_TYPE = "keen::VfxResource"
local COLOR_TYPE = 1392743968

local EFFECTS = {
    rayos = "df4962ec-1eaa-4c23-a8c1-0e5ac994f275",
    rayos_mano = "65d48539-68c2-4086-9669-82c0a135ca8f",
    fuego = "f850bc76-0efb-4c80-a77e-da0fd5dffd60",
    llama = "334d5410-faa3-4152-927b-98a64c2c992f",
    hielo = "8617bf03-e195-445e-9409-c89ff6a8a5f9",
    escarcha = "c9f7b398-169c-4a56-9495-750c35fc53a1",
    niebla = "8685a499-da08-4290-8f1c-686e34c85699",
    maldicion = "c5e00d19-2899-4832-9c4e-3d7b439049b0",
    niebla_muerte = "872542d9-4e97-47fa-a2aa-407a4a8479ac",
    luz = "86962dd8-fc0c-425a-a9d3-2495fd7029a4",
    acido = "a4898fc5-e1a3-475c-b7dc-688e35401f07",
    veneno = "d4ab4201-92bc-4323-9acd-b2f5dc583dc2",
    hueso = "fc09f4cf-2b3f-4ef4-b257-1cd1cdd4239f",
    curacion = "a3d8af6b-d34f-4b1a-b633-a0a02a3b3c30",
    drenaje = "87c8d367-7684-4d8e-9a78-a25a59cc269e",
    ectoplasma = "0fd40c96-e14c-4d61-ab15-b061394bbb30",
    tentaculos = "f29d3a6f-8a2b-4f18-9aa8-95ad1645198e",
    ardiendo = "85e60392-52e2-4488-8653-2d539c18c80c",
    aura_fuego = "0b12c979-65b2-43e1-80ab-e3bddb8f2f3e",
    aura_agua = "15736b49-aed2-4c9c-b147-bdbfd5eff580",
    sangre = "1603d364-10c1-4623-8c35-ba64884dd4eb",
    manos_brillantes = "54347d4a-6815-4ffc-a9f7-25f96ae6f41e",
    meteorico = "0f775769-5792-4db0-9f9c-2cbb92015073",
    aturdido = "6ab9bfa1-8849-4b64-9fb1-f240882c26e2",
}

local SPELLS = {
    VFX_Ability_Casting_Fire = "fuego",
    VFX_Ability_Casting_Fire_Small = "fuego",
    VFX_Ability_Casting_Fire_Flamethrower_TEST = "fuego",
    VFX_Ability_Casting_The_Flame = "llama",
    VFX_Ability_Casting_Summon_Skeleton = "llama",
    VFX_Ability_Casting_Ice = "hielo",
    VFX_Ability_Casting_Ice_Blizzard = "hielo",
    VFX_Ability_Casting_Lightning = "rayo",
    VFX_Ability_Casting_Lightning_Ball = "rayo",
    VFX_pvp_tnt2_laser_terraformer_casting = "rayo",
    VFX_Ability_Casting_Fog = "niebla",
    VFX_Ability_Fog_Meteor_Level_01 = "niebla",
    VFX_Ability_Casting_Light = "luz",
    VFX_Ability_Eternal_Lightburst = "luz",
    VFX_Ability_Casting_Acid = "acido",
    VFX_Ability_Casting_Acid_Eternal = "acido",
    VFX_Ability_Casting_Root_placeholder = "acido",
    VFX_Ability_Casting_Bone = "hueso",
    VFX_Ability_Bone_Channel_Level_2 = "hueso",
    VFX_Ability_Casting_Heal_Channel = "curacion",
    VFX_Ability_Casting_Heal_Channel_Eternal = "curacion",
    VFX_Ability_Casting_Chain_Heal_Eternal = "curacion",
    VFX_Ability_Casting_ManaDrain = "drenaje",
    VFX_Ability_Casting_Water = "agua",
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

local function num(v)
    if type(v) == "number" then return v end
    local inner = try(function() return v.value end)
    if type(inner) == "number" then return inner end
    return tonumber(tostring(v))
end

local function parse_color(text)
    if type(text) ~= "string" then return nil end
    local s = string.lower((text:gsub("%s", "")))
    if s == "" then return nil end
    s = NAMED_COLORS[s] or s
    s = s:gsub("^#", "")
    if not s:match("^%x%x%x%x%x%x$") then
        print(TAG .. " invalid color '" .. text .. "' ignored")
        return nil
    end
    return 0xFF000000 + tonumber(s:sub(5, 6), 16) * 65536 + tonumber(s:sub(3, 4), 16) * 256 + tonumber(s:sub(1, 2), 16)
end

local function color_list(entry)
    local out = {}
    if type(entry) == "table" then
        for _, c in ipairs(entry) do
            local v = parse_color(c)
            if v then out[#out + 1] = v end
        end
    else
        local v = parse_color(entry)
        if v then out[1] = v end
    end
    return out
end

local loaded, CONFIG = pcall(require, "m182_vfx_config")
if not loaded or type(CONFIG) ~= "table" then
    print(TAG .. " config not readable, using defaults: " .. tostring(CONFIG))
    CONFIG = {}
end

local INTENSITY = tonumber(CONFIG.intensidad) or 1
if INTENSITY < 1 then INTENSITY = 1 end
if INTENSITY > 10 then INTENSITY = 10 end
local DEFAULT_EFFECT = string.lower(tostring(CONFIG.efecto or "original"))
local DEFAULT_COLORS = color_list(CONFIG.color)
local PER_SPELL = type(CONFIG.hechizos) == "table" and CONFIG.hechizos or {}

local function choice_for(spell)
    local own = PER_SPELL[spell]
    if type(own) == "table" then
        local key = string.lower(tostring(own.efecto or DEFAULT_EFFECT))
        local colors = color_list(own.color)
        if #colors == 0 then colors = DEFAULT_COLORS end
        return key, colors
    elseif type(own) == "string" and own ~= "" then
        return string.lower(own), DEFAULT_COLORS
    end
    return DEFAULT_EFFECT, DEFAULT_COLORS
end

local prepared = {}
local prepared_count = 0

local function prepare(source_guid, key, colors)
    if prepared[source_guid] then return source_guid end
    local resource = try(function() return game.assets.get_resource(source_guid, VFX_TYPE, 0) end)
    if resource == nil then
        print(TAG .. " effect " .. key .. " (" .. source_guid .. ") not found")
        return nil
    end
    prepared[source_guid] = true
    prepared_count = prepared_count + 1
    local d = resource.data
    local amplified, painted = 0, 0
    if INTENSITY > 1 then
        for _, p in ipairs(d.particles) do
            local count = num(p.maxCount)
            if count and count > 0 then
                local target = math.floor(count * INTENSITY)
                if target > count and pcall(function() p.maxCount = target end) then amplified = amplified + 1 end
            end
        end
    end
    if #colors > 0 then
        local block = d.defaultParameterData.data
        for _, p in ipairs(d.parameters) do
            if num(p.typeId) == COLOR_TYPE then
                local index = math.floor((num(p.dataRange.start) or 0) / 4) + 1
                if index >= 1 and index <= 16 then
                    painted = painted + 1
                    block[index] = colors[((painted - 1) % #colors) + 1]
                end
            end
        end
    end
    print(string.format("%s effect %s (%s) particles x%s: %d, colors: %d", TAG, key, source_guid, tostring(INTENSITY), amplified, painted))
    return source_guid
end

local templates = try(function() return game.assets.get_resources_by_type("keen::ecs::TemplateResource") end) or {}
local patched, kept = 0, 0
for _, resource in ipairs(templates) do
    local data = resource.data
    local name = try(function() return data.name end)
    local spell = name and SPELLS[name]
    if spell then
        local key, colors = choice_for(spell)
        for _, component in ipairs(data.components) do
            if component.type == "keen::ecs::VfxComponentResource" and component.value ~= nil then
                local current = try(function() return component.value.vfx end)
                local source
                if key == "original" or key == "" then
                    source = current
                else
                    source = EFFECTS[key]
                    if source == nil then
                        print(TAG .. " unknown effect '" .. key .. "' for " .. spell .. ", keeping the original")
                        source = current
                        key = "original"
                    end
                end
                if source ~= nil and (key ~= "original" or INTENSITY > 1 or #colors > 0) then
                    local guid = prepare(source, key, colors)
                    if guid and pcall(function() component.value.vfx = guid end) then
                        patched = patched + 1
                        print(TAG .. " " .. name .. " (" .. spell .. ") = " .. key)
                    end
                else
                    kept = kept + 1
                end
            end
        end
    end
end

print(string.format("%s done: spell visuals changed=%d unchanged=%d effects prepared=%d", TAG, patched, kept, prepared_count))
