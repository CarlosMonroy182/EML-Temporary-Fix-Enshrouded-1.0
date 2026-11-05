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
    local seen, zone, reach, materials, gathering = {}, 0, 0, 0, 0
    if type(items) ~= "table" then return end
    local reach_limit = tonumber(config.build_reach) or 0
    for _, resource in ipairs(items) do
        pcall(function()
            local equipment = resource.data.equipment
            local slot = tostring(equipment.slot)
            if not SLOTS[slot] then return end
            local debug_name = tostring(resource.data.debugName):lower()
            if debug_name:find("pickaxe", 1, true) or debug_name:find("goldsieve", 1, true) or debug_name:find("_axe_", 1, true) then
                gathering = gathering + 1
                return
            end
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
    print(string.format("%s build items %d: build zone not required %d, material check off %d, reach raised %d, gathering tools left alone %d", TAG, total, zone, materials, reach, gathering))
end

local function placed_entities()
    local set = {}
    local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
    if type(items) ~= "table" then return set end
    for _, resource in ipairs(items) do
        pcall(function()
            local equipment = resource.data.equipment
            if tostring(equipment.slot) == "BuildTool" then
                set[tostring(equipment.placedEntity):lower()] = true
            end
        end)
    end
    return set
end

local function perm_text(v)
    local ok, out = pcall(function()
        local parts = {}
        for _, x in ipairs(v) do parts[#parts + 1] = tostring(x) end
        return "{" .. table.concat(parts, ",") .. "}"
    end)
    if ok then return out end
    return tostring(v)
end

local function last_lock(resource)
    local found
    for _, c in ipairs(resource.data.components) do
        if tostring(c.type) == "keen::ecs::InteractionLock" then found = c end
    end
    return found
end

local function patch_doors()
    local placed = placed_entities()
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
    local wanted = config.door_lock_permission or "CanAccessInventories"
    local donor_before = perm_text(try(function() return donor.value.requiredPermission end))
    local locked, stations = 0, 0
    local extra, untoggled = {}, {}
    local retargeted, sample = 0, nil
    local function retarget(resource)
        local c = last_lock(resource)
        if c == nil then return end
        local ok = pcall(function() c.value.requiredPermission = { wanted } end)
        if ok then
            retargeted = retargeted + 1
            if sample == nil then sample = perm_text(try(function() return c.value.requiredPermission end)) end
        end
    end
    for _, resource in ipairs(templates) do
        pcall(function()
            local name = tostring(resource.data.name)
            local lower = name:lower()
            local buildable = placed[tostring(resource.guid):lower()]
            local crafted = name:find("Door") and name:find("Crafted")
            local doorlike = buildable and (lower:find("door") or lower:find("gate") or lower:find("portcullis")) and not lower:find("frame")
            local door = (crafted or doorlike) and not name:find("DEPRECATED")
            local station = config.stations_need_permission ~= false and buildable and not door and not name:find("FlameAltar")
            if not door and not station then return end
            local toggle, lock, craft = false, false, false
            for _, c in ipairs(resource.data.components) do
                local t = tostring(c.type)
                if t == "keen::ecs::InteractionToggle" then toggle = true end
                if t == "keen::ecs::InteractionLock" then lock = true end
                if t == "keen::ecs::CraftingInteraction" then craft = true end
            end
            if lock then return end
            if door and toggle then
                table.insert(resource.data.components, donor)
                retarget(resource)
                locked = locked + 1
                if not crafted then table.insert(extra, name) end
            elseif door then
                table.insert(untoggled, name)
            elseif station and craft then
                table.insert(resource.data.components, donor)
                retarget(resource)
                stations = stations + 1
            end
        end)
    end
    print(string.format("%s player doors that need the building permission: %d, crafting stations: %d", TAG, locked, stations))
    print(string.format("%s door and station lock needs %s (lock source had %s, now %s; %d locks changed, first reads %s)", TAG, wanted, donor_before, perm_text(try(function() return donor.value.requiredPermission end)), retargeted, tostring(sample)))
    if #extra > 0 then print(TAG .. " doors added from the building menu: " .. table.concat(extra, ", ")) end
    if #untoggled > 0 then print(TAG .. " door pieces without an open/close interaction (unchanged): " .. table.concat(untoggled, ", ")) end
end

local function patch_dismantle()
    local placed = placed_entities()
    local templates = try(function() return game.assets.get_resources_by_type("keen::ecs::TemplateResource") end)
    if type(templates) ~= "table" then return end
    local donor
    local changed, added = 0, 0
    for _, resource in ipairs(templates) do
        pcall(function()
            for _, c in ipairs(resource.data.components) do
                if tostring(c.type) == "keen::ecs::DismantleOverride" then
                    local d = c.value or c.data or c
                    if donor == nil and d.overrideMethod == false then donor = c end
                    if d.ignoreBuildZoneChecks ~= true then
                        d.ignoreBuildZoneChecks = true
                        changed = changed + 1
                    end
                end
            end
        end)
    end
    if donor == nil then
        print(TAG .. " dismantle source not found")
        return
    end
    for _, resource in ipairs(templates) do
        pcall(function()
            if not placed[tostring(resource.guid):lower()] then return end
            for _, c in ipairs(resource.data.components) do
                if tostring(c.type) == "keen::ecs::DismantleOverride" then return end
            end
            table.insert(resource.data.components, donor)
            added = added + 1
        end)
    end
    print(string.format("%s dismantling outside building areas: %d pieces changed, %d added", TAG, changed, added))
end

patch_items()
if config.doors_need_permission ~= false then
    patch_doors()
end
if config.dismantle_anywhere ~= false then
    patch_dismantle()
end
