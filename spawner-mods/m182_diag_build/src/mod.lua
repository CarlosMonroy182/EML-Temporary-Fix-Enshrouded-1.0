local TAG = "[m182-build-test]"

local function try(f)
    local ok, v = pcall(f)
    if ok then return v end
    return nil
end

local config = try(function() return require("m182_diag_build_config") end) or {}

local SLOTS = {
    BuildTool = true,
    Tool = true,
    TerrainMaterial = true,
    BlueprintMaterial_Default = true,
    BlueprintMaterial_Roof = true,
    OvergrowthMaterial = true,
}

local function patch_items()
    local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
    local seen, zone, reach, materials = {}, 0, 0, 0
    if type(items) ~= "table" then return end
    local reach_limit = tonumber(config.build_reach) or 0
    for _, resource in ipairs(items) do
        pcall(function()
            local equipment = resource.data.equipment
            local slot = tostring(equipment.slot)
            if not SLOTS[slot] then return end
            local cursor = equipment.cursorConfig
            local builds = slot ~= "Tool" or cursor.checkRequiredMaterialsOnPrimaryAction == true or cursor.checkRequiredMaterialsOnSecondaryAction == true
            seen[slot] = (seen[slot] or 0) + 1
            if config.build_anywhere ~= false and equipment.buildZoneRequired == true then
                equipment.buildZoneRequired = false
                zone = zone + 1
            end
            if not builds then return end
            if config.free_materials_check ~= false then
                if cursor.checkRequiredMaterialsOnPrimaryAction == true then
                    cursor.checkRequiredMaterialsOnPrimaryAction = false
                    materials = materials + 1
                end
                if cursor.checkRequiredMaterialsOnSecondaryAction == true then
                    cursor.checkRequiredMaterialsOnSecondaryAction = false
                end
            end
            if reach_limit > 0 then
                if equipment.maxDistance < reach_limit then
                    equipment.maxDistance = reach_limit
                    reach = reach + 1
                end
                if equipment.maxHorizontalDistance < reach_limit then
                    equipment.maxHorizontalDistance = reach_limit
                end
            end
        end)
    end
    local total = 0
    for _, n in pairs(seen) do total = total + n end
    print(string.format("%s build items %d: build zone not required %d, material check off %d, reach raised %d", TAG, total, zone, materials, reach))
end

local function patch_doors()
    local templates = try(function() return game.assets.get_resources_by_type("keen::ecs::TemplateResource") end)
    if type(templates) ~= "table" then return end
    local donor
    for _, resource in ipairs(templates) do
        pcall(function()
            if donor == nil and tostring(resource.data.name) == "WaterDrain_ground_base_PLAYER" then
                for _, c in ipairs(resource.data.components) do
                    if tostring(c.type) == "keen::ecs::InteractionLock" then donor = c end
                end
            end
        end)
    end
    if donor == nil then
        print(TAG .. " door lock source not found, doors unchanged")
        return
    end
    local locked, failed = 0, 0
    for _, resource in ipairs(templates) do
        pcall(function()
            local name = tostring(resource.data.name)
            if not (name:find("Door") and name:find("Crafted")) or name:find("DEPRECATED") then return end
            local toggle, lock = false, false
            for _, c in ipairs(resource.data.components) do
                local t = tostring(c.type)
                if t == "keen::ecs::InteractionToggle" then toggle = true end
                if t == "keen::ecs::InteractionLock" then lock = true end
            end
            if toggle and not lock then
                table.insert(resource.data.components, donor)
                locked = locked + 1
            end
        end)
    end
    print(string.format("%s player doors that need the building permission: %d", TAG, locked))
end

patch_items()
if config.doors_need_permission ~= false then
    patch_doors()
end
