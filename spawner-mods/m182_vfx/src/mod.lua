local TAG = "[m182-vfx]"
local TEMPLATE_TYPE = "keen::ecs::TemplateResource"
local VFX_TYPE = "keen::VfxResource"
local BASE = "eee801de-8da6-4e11-b5f7-159205ffb139"

local ELEMENTS = {
    { n = 1, name = "Lightning", vfx = "65d48539-68c2-4086-9669-82c0a135ca8f" },
    { n = 2, name = "Fire", vfx = "85e60392-52e2-4488-8653-2d539c18c80c" },
    { n = 3, name = "Ice", vfx = "c9f7b398-169c-4a56-9495-750c35fc53a1" },
    { n = 4, name = "Shroud", vfx = "c5e00d19-2899-4832-9c4e-3d7b439049b0" },
    { n = 5, name = "Healing", vfx = "54347d4a-6815-4ffc-a9f7-25f96ae6f41e" },
    { n = 6, name = "Poison", vfx = "d4ab4201-92bc-4323-9acd-b2f5dc583dc2" },
    { n = 7, name = "Bone", vfx = "f29d3a6f-8a2b-4f18-9aa8-95ad1645198e" },
    { n = 8, name = "Arcane", vfx = "0f775769-5792-4db0-9f9c-2cbb92015073" },
    { n = 9, name = "Physical", vfx = "1603d364-10c1-4623-8c35-ba64884dd4eb" },
    { n = 10, name = "Water", vfx = "15736b49-aed2-4c9c-b147-bdbfd5eff580" },
    { n = 11, name = "Deathfog", vfx = "872542d9-4e97-47fa-a2aa-407a4a8479ac" },
    { n = 12, name = "Ectoplasm", vfx = "0fd40c96-e14c-4d61-ab15-b061394bbb30" },
    { n = 13, name = "Stun", vfx = "6ab9bfa1-8849-4b64-9fb1-f240882c26e2" },
    { n = 14, name = "FireRing", vfx = "0b12c979-65b2-43e1-80ab-e3bddb8f2f3e" },
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

local base = try(function() return game.assets.get_resource(BASE, TEMPLATE_TYPE, 0) end)
if base == nil then
    print(TAG .. " base effect template not found, glow effects off")
    return
end

local created, failed = 0, 0
for _, e in ipairs(ELEMENTS) do
    local guid = string.format("182e1e00-0000-4000-8000-%012x", e.n)
    local ok, err = pcall(function()
        local vfx = game.assets.get_resource(e.vfx, VFX_TYPE, 0)
        if vfx == nil then error("effect " .. e.vfx .. " not found") end
        local defaults = vfx.data.defaultParameterData.data
        local existing = try(function() return game.assets.get_resource(guid, TEMPLATE_TYPE, 0) end)
        local copy = existing or game.assets.create_resource(base.data, TEMPLATE_TYPE, guid, 0)
        local data = copy.data
        data.name = "M182_VFX_Glow_" .. e.name
        for _, c in ipairs(data.components) do
            local t = tostring(c.type)
            if t == "keen::ecs::VfxComponentResource" then
                c.value.vfx = e.vfx
            elseif t == "keen::ecs::VfxParametersTemplateComponent" then
                local block = c.value.parameters.data
                for i = 1, 16 do block[i] = unsigned(num(defaults[i]) or 0) end
            end
        end
    end)
    if ok then
        created = created + 1
    else
        failed = failed + 1
        print(TAG .. " glow effect " .. e.name .. " failed: " .. tostring(err))
    end
end

print(string.format("%s glow effects ready=%d failed=%d", TAG, created, failed))
