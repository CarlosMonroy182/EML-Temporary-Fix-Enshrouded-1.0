local TAG = "[m182-divine]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local GLOW = "1c3281da-e7d4-4b41-9033-c85daec7f72a"
local BASE = "eee801de-8da6-4e11-b5f7-159205ffb139"
local LIFE_TIME = "keen::ecs::LifeTime"
local LONG_LIFE_NS = 1000000000000000000

local function try(f)
    local ok, r = pcall(f)
    if ok then return r end
    return nil
end

local function find(components, type_name)
    for _, c in ipairs(components) do
        if tostring(c.type) == type_name then return c end
    end
    return nil
end

local glow = try(function() return game.assets.get_resource(GLOW, TEMPLATE_TYPE, 0) end)
local base = try(function() return game.assets.get_resource(BASE, TEMPLATE_TYPE, 0) end)
if glow == nil or base == nil then
    print(TAG .. " templates not found, nothing changed")
    return
end

local components = glow.data.components
if find(components, LIFE_TIME) == nil then
    local part = find(base.data.components, LIFE_TIME)
    if part == nil then
        print(TAG .. " lifetime part not found, nothing changed")
        return
    end
    table.insert(components, part)
end

local life = find(components, LIFE_TIME)
local set = 0
if life ~= nil then
    if pcall(function() life.value.lifeTime.value = LONG_LIFE_NS end) then set = set + 1 end
    if pcall(function() life.value.enforceDestroy = true end) then set = set + 1 end
end
print(string.format("%s teleport glow ready (lifetime part %s, %d values set)", TAG, life ~= nil and "present" or "missing", set))
