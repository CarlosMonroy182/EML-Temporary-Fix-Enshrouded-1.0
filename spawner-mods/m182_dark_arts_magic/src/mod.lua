local TAG = "[m182-dark-arts]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local ITEM_TYPE = "keen::ItemInfo"

local PILLAR_RADIUS_M = 15.0

local HIT_RADIUS = 0.9
local SWEEP_DAMAGE = false
local RED = 255
local GREEN = 40
local BLUE = 20

local PILLAR_SPELL = "a4d836d6-638c-4ebe-9cc9-f2b3c32d4b00"
local PILLAR_RADIUS_CONFIGS = { [1674724000] = true, [1381503774] = true }
local BEAM = "a0f608e8-f3d5-4e09-85d9-a03095ac9a3a"
local ORIGINAL_HIT_RADIUS = 0.2

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

local function value_of(x)
    local v = try(function() return x.value end)
    if v ~= nil then return v end
    return x
end

local function close(a, b)
    return type(a) == "number" and math.abs(a - b) < 0.001
end

local function color(r, g, b)
    return 4278190080 + b * 65536 + g * 256 + r
end

local function pillar()
    local spell = try(function() return game.assets.get_resource(PILLAR_SPELL, ITEM_TYPE, 0) end)
    if spell == nil then return 0 end
    local changed = 0
    for _, entry in ipairs(spell.data.impactValues.simple) do
        local v = entry.value
        local id = try(function() return v.configId.value end)
        if PILLAR_RADIUS_CONFIGS[id] and tostring(entry.type) == "keen::impact::FloatImpactConfig" then
            if pcall(function() v.value = PILLAR_RADIUS_M end) then changed = changed + 1 end
        end
    end
    return changed
end

local function beam()
    local resource = try(function() return game.assets.get_resource(BEAM, TEMPLATE_TYPE, 0) end)
    if resource == nil then return 0, 0 end
    local hit, visual = 0, 0
    for _, c in ipairs(resource.data.components) do
        local t = tostring(c.type)
        if t == "keen::ecs::ColliderResourceComponent" then
            for _, collider in ipairs(c.value.colliders) do
                for _, entry in ipairs(collider.dataArray) do
                    local data = value_of(entry)
                    local shape = try(function() return data.shape end)
                    local r = shape and num(shape.radius)
                    if shape ~= nil and close(r, ORIGINAL_HIT_RADIUS) then
                        if pcall(function() shape.radius = HIT_RADIUS end) then
                            hit = hit + 1
                            if not SWEEP_DAMAGE then pcall(function() data.continuous = false end) end
                        end
                    end
                end
            end
        elseif t == "keen::ecs::VfxParametersTemplateComponent" then
            local block = c.value.parameters.data
            block[4] = color(RED, GREEN, BLUE)
            visual = visual + 1
        end
    end
    return hit, visual
end

local fire = pillar()
local hit, visual = beam()
print(string.format("%s pillar values=%d (radius %.1f m); veil beam hit areas=%d looks=%d", TAG, fire, PILLAR_RADIUS_M, hit, visual))
