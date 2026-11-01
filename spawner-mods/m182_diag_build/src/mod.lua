local TAG = "[m182-build-test]"

local function try(f)
    local ok, v = pcall(f)
    if ok then return v end
    return nil
end

local config = try(function() return require("m182_diag_build_config") end) or {}
if config.build_anywhere == false then
    print(TAG .. " build_anywhere is off, nothing changed")
    return
end

local SLOTS = {
    BuildTool = true,
    Tool = true,
    TerrainMaterial = true,
    BlueprintMaterial_Default = true,
    BlueprintMaterial_Roof = true,
    OvergrowthMaterial = true,
}

local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
local seen, changed = {}, {}
if type(items) == "table" then
    for _, resource in ipairs(items) do
        pcall(function()
            local equipment = resource.data.equipment
            local slot = tostring(equipment.slot)
            if not SLOTS[slot] then return end
            seen[slot] = (seen[slot] or 0) + 1
            if equipment.buildZoneRequired == true then
                equipment.buildZoneRequired = false
                if equipment.buildZoneRequired == false then changed[slot] = (changed[slot] or 0) + 1 end
            end
        end)
    end
end
for slot in pairs(SLOTS) do
    print(string.format("%s %s: %d items, %d no longer need a build zone", TAG, slot, seen[slot] or 0, changed[slot] or 0))
end
