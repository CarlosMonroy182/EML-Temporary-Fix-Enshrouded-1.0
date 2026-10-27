local TAG = "[m182-beam]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local BEAM = "a0f608e8-f3d5-4e09-85d9-a03095ac9a3a"

local HIT_RADIUS = 1.5
local VISUAL_RADIUS = 1.5
local LIGHT_RADIUS = 12.0

local ORIGINAL_HIT_RADIUS = 0.2
local ORIGINAL_VISUAL_RADIUS = 0.1
local ORIGINAL_LIGHT_RADIUS = 3.0

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

local function bits(x)
    return (string.unpack("<I4", string.pack("<f", x)))
end

local function float_of(n)
    return (string.unpack("<f", string.pack("<I4", math.floor(n) % 4294967296)))
end

local function value_of(x)
    local v = try(function() return x.value end)
    if v ~= nil then return v end
    return x
end

local function close(a, b)
    return type(a) == "number" and math.abs(a - b) < 0.001
end

local resource = try(function() return game.assets.get_resource(BEAM, TEMPLATE_TYPE, 0) end)
if resource == nil then
    print(TAG .. " beam template not found, nothing changed")
    return
end

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
                    local ok = pcall(function() shape.radius = HIT_RADIUS end)
                    if ok then hit = hit + 1 end
                end
            end
        end
    elseif t == "keen::ecs::VfxParametersTemplateComponent" then
        local block = c.value.parameters.data
        local radius, light = num(block[1]), num(block[2])
        if radius and light and close(float_of(radius), ORIGINAL_VISUAL_RADIUS) and close(float_of(light), ORIGINAL_LIGHT_RADIUS) then
            block[1] = bits(VISUAL_RADIUS)
            block[2] = bits(LIGHT_RADIUS)
            visual = visual + 1
        end
    end
end

print(string.format("%s veil beam: hit areas=%d looks=%d (hit radius %.2f m, beam radius %.2f m)", TAG, hit, visual, HIT_RADIUS, VISUAL_RADIUS))
