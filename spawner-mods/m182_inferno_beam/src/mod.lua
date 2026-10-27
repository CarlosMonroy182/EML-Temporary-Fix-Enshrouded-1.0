local TAG = "[m182-beam]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local BEAM = "a0f608e8-f3d5-4e09-85d9-a03095ac9a3a"

local HIT_RADIUS = 1.5

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

local resource = try(function() return game.assets.get_resource(BEAM, TEMPLATE_TYPE, 0) end)
if resource == nil then
    print(TAG .. " beam template not found, nothing changed")
    return
end

local hit = 0
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
    end
end

print(string.format("%s veil beam: hit areas=%d (hit radius %.2f m)", TAG, hit, HIT_RADIUS))
