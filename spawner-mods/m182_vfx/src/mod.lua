local TAG = "[m182-vfx]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local VFX_TYPE = "keen::VfxResource"
local BASE = "eee801de-8da6-4e11-b5f7-159205ffb139"
local SERVER_PARTS = { "keen::ecs::DoNotSave", "keen::ecs::LifeTime" }

local GLOWS = {
    { name = "Lightning", carrier = "ad949f69-aadd-4f5e-97dc-1db3afc5ffde", vfx = "65d48539-68c2-4086-9669-82c0a135ca8f" },
    { name = "Fire", carrier = "21f9f3e9-433f-48d7-b605-c0f2ad1ed5cf", vfx = "85e60392-52e2-4488-8653-2d539c18c80c" },
    { name = "Ice", carrier = "d6d33ef8-8681-47b6-a429-04c00666cca1", vfx = "c9f7b398-169c-4a56-9495-750c35fc53a1" },
    { name = "Shroud", carrier = "f7bc7106-ac91-45da-9d93-056bddf0c8d0", vfx = "c5e00d19-2899-4832-9c4e-3d7b439049b0" },
    { name = "Healing", carrier = "04c22eef-5e56-4a6e-b340-ee07366862f5", vfx = "54347d4a-6815-4ffc-a9f7-25f96ae6f41e" },
    { name = "Poison", carrier = "c7fd6895-e278-4b10-abb2-d4b2f882a134", vfx = "d4ab4201-92bc-4323-9acd-b2f5dc583dc2" },
    { name = "Bone", carrier = "cc9d30cb-cf05-4baf-ab07-77586516dc3c", vfx = "f29d3a6f-8a2b-4f18-9aa8-95ad1645198e" },
    { name = "Arcane", carrier = "45d8ce2a-0b08-44d4-b47e-6a849a149279", vfx = "0f775769-5792-4db0-9f9c-2cbb92015073" },
    { name = "Physical", carrier = "0b5af7c4-5183-4e61-99a2-12535f90d6d0", vfx = "1603d364-10c1-4623-8c35-ba64884dd4eb" },
    { name = "Water", carrier = "25662ec7-c5cd-4595-ab2c-9ff46b8dbcd6", vfx = "15736b49-aed2-4c9c-b147-bdbfd5eff580" },
    { name = "Deathfog", carrier = "be24c385-8b5e-4ea7-9827-48eaec9c5209", vfx = "872542d9-4e97-47fa-a2aa-407a4a8479ac" },
    { name = "Ectoplasm", carrier = "132d2794-96a7-4e45-b80a-fd8d14810ac7", vfx = "0fd40c96-e14c-4d61-ab15-b061394bbb30" },
    { name = "Stun", carrier = "f56a56dd-a068-43e9-8ec4-f4fdbb68846b", vfx = "6ab9bfa1-8849-4b64-9fb1-f240882c26e2" },
    { name = "FireRing", carrier = "73500153-66c8-420d-a99e-a7341daa6e32", vfx = "0b12c979-65b2-43e1-80ab-e3bddb8f2f3e" },
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

local function unsigned(n)
    n = math.floor(n)
    if n < 0 then n = n + 4294967296 end
    return n
end

local function find(components, type_name)
    for _, c in ipairs(components) do
        if tostring(c.type) == type_name then return c end
    end
    return nil
end

local base = try(function() return game.assets.get_resource(BASE, TEMPLATE_TYPE, 0) end)
if base == nil then
    print(TAG .. " base effect template not found, glow effects off")
    return
end

local ready, looks, failed = 0, 0, 0
for _, g in ipairs(GLOWS) do
    local ok, err = pcall(function()
        local carrier = game.assets.get_resource(g.carrier, TEMPLATE_TYPE, 0)
        local components = carrier.data.components
        for _, part in ipairs(SERVER_PARTS) do
            if find(components, part) == nil then
                table.insert(components, find(base.data.components, part))
            end
        end
        for _, part in ipairs(SERVER_PARTS) do
            if find(carrier.data.components, part) == nil then error("could not add " .. part) end
        end
        ready = ready + 1
        local vfx = try(function() return game.assets.get_resource(g.vfx, VFX_TYPE, 0) end)
        local defaults = vfx and try(function() return vfx.data.defaultParameterData.data end)
        if vfx == nil then return end
        for _, c in ipairs(carrier.data.components) do
            local t = tostring(c.type)
            if t == "keen::ecs::VfxComponentResource" then
                c.value.vfx = g.vfx
            elseif t == "keen::ecs::VfxComponent" then
                c.value.slot = "Chest"
                c.value.attachedToEntity = "Self"
                c.value.attachToParent = false
            elseif t == "keen::ecs::VfxParametersTemplateComponent" and defaults ~= nil then
                local block = c.value.parameters.data
                for i = 1, 16 do block[i] = unsigned(num(defaults[i]) or 0) end
            end
        end
        looks = looks + 1
    end)
    if not ok then
        failed = failed + 1
        print(TAG .. " glow " .. g.name .. " failed: " .. tostring(err))
    end
end

print(string.format("%s glow carriers ready=%d looks=%d failed=%d", TAG, ready, looks, failed))
