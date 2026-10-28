local TAG = "[m182-pillar]"
local ITEM_TYPE = "keen::ItemInfo"
local SPELL = "a4d836d6-638c-4ebe-9cc9-f2b3c32d4b00"
local RADIUS_CONFIG = 1674724000
local ZONE = "a77023ba-4010-466b-8554-61fd3e9a621b"
local ZONE_RADIUS_M = 3.0

local RADIUS_M = 15.0
local DURATION_S = 15.0
local ORIGINAL_DURATION_NS = 6000000000

local function try(f)
    local ok, r = pcall(f)
    if ok then return r end
    return nil
end

local resource = try(function() return game.assets.get_resource(SPELL, ITEM_TYPE, 0) end)
if resource == nil then
    print(TAG .. " spell not found, nothing changed")
    return
end

local changed = 0
for _, entry in ipairs(resource.data.impactValues.simple) do
    local v = entry.value
    local id = try(function() return v.configId.value end)
    if id == RADIUS_CONFIG then
        local kind = tostring(entry.type)
        if kind == "keen::impact::FloatImpactConfig" then
            if pcall(function() v.value = RADIUS_M end) then changed = changed + 1 end
        elseif kind == "keen::impact::TimeImpactConfig" then
            local ns = try(function() return v.value.value end)
            if ns == ORIGINAL_DURATION_NS then
                if pcall(function() v.value.value = math.floor(DURATION_S * 1000000000) end) then changed = changed + 1 end
            end
        end
    end
end

for _, entry in ipairs(resource.data.uiValues) do
    local id = try(function() return entry.locaId.value end)
    if id == 609674671 and entry.value == 6000 then
        if pcall(function() entry.value = math.floor(DURATION_S * 1000) end) then changed = changed + 1 end
    end
end

local zone = try(function() return game.assets.get_resource(ZONE, "keen::ecs::TemplateResource", 0) end)
if zone ~= nil then
    local scale = RADIUS_M / ZONE_RADIUS_M
    for _, c in ipairs(zone.data.components) do
        local t = tostring(c.type)
        if t == "keen::ecs::CurrentTransform" or t == "keen::ecs::RenderTransform" then
            local s = c.value.transform.scale
            if pcall(function() s.x = scale; s.y = scale; s.z = scale end) then changed = changed + 1 end
        elseif t == "keen::ecs::LifeTime" then
            if pcall(function() c.value.lifeTime.value = math.floor(DURATION_S * 1000000000) end) then changed = changed + 1 end
        end
    end
end

print(string.format("%s pillar: %d values changed (radius %.1f m, %.1f s)", TAG, changed, RADIUS_M, DURATION_S))
