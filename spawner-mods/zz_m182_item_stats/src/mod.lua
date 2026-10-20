local loaded, DATA = pcall(require, "m182_data")
if not loaded or type(DATA) ~= "table" then DATA = {} end
local OVERRIDES = type(DATA.overrides) == "table" and DATA.overrides or {}
local LEVELS = type(DATA.levels) == "table" and DATA.levels or {}
local BALANCE = type(DATA.balance) == "table" and DATA.balance or {}
for _, o in pairs(OVERRIDES) do
    if type(o) == "table" then
        if type(o.dmg) == "table" then
            local named = {}
            for i, key in ipairs({"speedMod","dmgMod","physicalCutDamage","physicalPierceDamage","physicalBluntDamage","magicalFireDamage","magicalIceDamage","magicalFogDamage","magicalLightningDamage","healing","poisonDamage","woodDamage","stoneDamage","metalDamage","explosionDamage"}) do if o.dmg[i] ~= nil then named[key] = o.dmg[i] end end
            o.dmg = named
        end
        if type(o.arm) == "table" then
            local named = {}
            for i, key in ipairs({"physical","blunt","pierce","cut","magical","fire","ice","fog","lightning"}) do if o.arm[i] ~= nil then named[key] = o.arm[i] end end
            o.arm = named
        end
    end
end
local FREE_SKILLS = BALANCE["freeSkills"] == 1
BALANCE["freeSkills"] = nil
local WIDEN = tonumber(BALANCE["itemLevelCap"]) or 0
if WIDEN <= 55 then WIDEN = 0 end

local function try(f)
    local ok, r = pcall(f)
    if ok then return r end
    return nil
end

local function hash_of(v)
    if v == nil then return nil end
    local t = type(v)
    if t == "number" then return v end
    if t == "table" then return v.value or v.id or v.hash end
    local inner = try(function() return v.value end)
    if type(inner) == "number" then return inner end
    return tonumber(tostring(v))
end

local INTEGER_KEYS = {
    durabilityLossCap = true, playerBaseHealth = true, playerHealthPerAP = true, playerBaseMana = true,
    playerManaPerAP = true, playerBaseStamina = true, playerStaminaPerAP = true, itemLevelCap = true,
    playerLevelCap = true, hookShotCostBase = true, maximumAnimalsInWorld = true,
    woodDamage = true, stoneDamage = true, metalDamage = true, explosionDamage = true,
}

local function put(container, key, value)
    if type(value) ~= "number" or value ~= value or value == math.huge or value == -math.huge then return false end
    if INTEGER_KEYS[key] then
        value = math.floor(value + 0.5)
        if value < 0 then value = 0 end
    end
    return pcall(function() container[key] = value end)
end

local applied, failed = 0, 0
print("[m182-item-stats] start")

local function unsigned(id)
    if type(id) ~= "number" then return nil end
    id = math.floor(id)
    if id < 0 then id = id + 4294967296 end
    return id
end

local function close(a, b)
    if type(a) ~= "number" or type(b) ~= "number" then return false end
    return math.abs(a - b) <= 0.001 + math.abs(b) * 0.0001
end

local function write_group(container, values, label, report)
    if container == nil then
        failed = failed + 1
        report.missing = report.missing + 1
        return
    end
    for k, v in pairs(values) do
        local target = container
        if label == "dmg" and k ~= "speedMod" and k ~= "dmgMod" then target = try(function() return container.distribution end) end
        if label == "arm" then target = try(function() return container.distribution end) end
        if target ~= nil and put(target, k, v) then
            applied = applied + 1
            report.written = report.written + 1
            local back = try(function() return target[k] end)
            local expected = v
            if INTEGER_KEYS[k] then expected = math.floor(v + 0.5) end
            if not close(back, expected) then
                report.mismatch = report.mismatch + 1
                if report.first == nil then report.first = string.format("%s=%s expected %s", k, tostring(back), tostring(expected)) end
            end
        else
            failed = failed + 1
            report.missing = report.missing + 1
        end
    end
end

local found = {}
local levelsDone = {}
local widened = 0
local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
if items ~= nil then
    pcall(function()
        for _, resource in ipairs(items) do
            pcall(function()
                local data = resource.data
                local id = unsigned(hash_of(data.itemId))
                local range = id and LEVELS[id]
                if WIDEN > 0 then
                    local wspan = try(function() return data.itemLevelRange end)
                    local wlo = try(function() return wspan.minLevel end)
                    local whi = try(function() return wspan.maxLevel end)
                    if type(wlo) == "number" and type(whi) == "number" and whi > wlo and whi < WIDEN and put(wspan, "maxLevel", WIDEN) then
                        widened = widened + 1
                    end
                end
                if range ~= nil then
                    local span = try(function() return data.itemLevelRange end)
                    local lo = try(function() return span.minLevel end)
                    local hi = try(function() return span.maxLevel end)
                    local wrote = false
                    if span ~= nil and type(lo) == "number" and type(hi) == "number" then
                        local newLo, newHi = math.min(lo, range[1]), math.max(hi, range[2])
                        if newLo ~= lo then wrote = put(span, "minLevel", newLo) end
                        if newHi ~= hi then wrote = put(span, "maxLevel", newHi) end
                        print(string.format("[m182-item-stats] item %d level range %s-%s -> %s-%s (%s)", id, tostring(lo), tostring(hi),
                            tostring(try(function() return span.minLevel end)), tostring(try(function() return span.maxLevel end)), wrote and "written" or "unchanged"))
                    else
                        print(string.format("[m182-item-stats] item %d has no readable level range", id))
                    end
                    levelsDone[id] = true
                end
                local o = id and OVERRIDES[id]
                if o == nil then return end
                local report = { written = 0, mismatch = 0, missing = 0 }
                if o.dmg ~= nil then write_group(try(function() return data.damageSetup end), o.dmg, "dmg", report) end
                if o.arm ~= nil then write_group(try(function() return data.armorSetup end), o.arm, "arm", report) end
                found[id] = (found[id] or 0) + 1
                print(string.format("[m182-item-stats] item %d (%s): wrote %d, not written %d, read-back mismatches %d%s",
                    id, tostring(try(function() return data.debugName end)), report.written, report.missing, report.mismatch,
                    report.first and (" first: " .. report.first) or ""))
            end)
        end
    end)
end
for id in pairs(OVERRIDES) do
    if found[id] == nil then print(string.format("[m182-item-stats] item %d not found in the game data", id)) end
end
for id in pairs(LEVELS) do
    if levelsDone[id] == nil then print(string.format("[m182-item-stats] item %d (level range) not found in the game data", id)) end
end

print(string.format("[m182-item-stats] items done applied=%d failed=%d widened=%d", applied, failed, widened))

if FREE_SKILLS then
    local nodes, freed = 0, 0
    local trees = try(function() return game.assets.get_resources_by_type("keen::SkillTreeResource") end)
    if type(trees) == "table" then
        for _, tree in ipairs(trees) do
            pcall(function()
                for _, node in ipairs(tree.data.nodes) do
                    nodes = nodes + 1
                    if put(node, "costs", 0) then freed = freed + 1 end
                end
            end)
        end
    end
    print(string.format("[m182-item-stats] skills free of cost: %d of %d nodes", freed, nodes))
end

local tables = try(function() return game.assets.get_resources_by_type("keen::BalancingTable") end)
if type(tables) == "table" and #tables > 0 then
    local d = tables[1].data
    for key, value in pairs(BALANCE) do
        local rarity = string.match(key, "^rarityDurabilityMod%.(%a+)$")
        local ok
        if rarity then
            ok = put(d.rarityDurabilityMod, rarity, value)
        else
            ok = put(d, key, value)
        end
        if ok then applied = applied + 1 else failed = failed + 1 end
        local back = try(function()
            if rarity then return d.rarityDurabilityMod[rarity] end
            return d[key]
        end)
        print(string.format("[m182-item-stats] balance %s = %s (%s)", key, tostring(back), ok and "written" or "not written"))
    end
end

print(string.format("[m182-item-stats] values applied=%d failed=%d", applied, failed))

local CUSTOM_FILE = "m182_custom.txt"
local CUSTOM_SKIP_FILE = "m182_custom.skipped"
local CUSTOM_DMG_KEYS = { "speedMod", "dmgMod", "physicalCutDamage", "physicalPierceDamage", "physicalBluntDamage", "magicalFireDamage", "magicalIceDamage", "magicalFogDamage", "magicalLightningDamage", "healing", "poisonDamage", "woodDamage", "stoneDamage", "metalDamage", "explosionDamage" }
local CUSTOM_ARM_KEYS = { "physical", "blunt", "pierce", "cut", "magical", "fire", "ice", "fog", "lightning" }

local function custom_id(base, slot)
    return (base + slot * 2654435761 + 182) % 4294967296
end

local function custom_guid(base, slot)
    local a, b, _, _, e = string.match(tostring(base), "^(%x+)%-(%x+)%-(%x+)%-(%x+)%-(%x+)$")
    if a == nil then return nil end
    local guid = string.format("%s-%s-5182-%04x-%s", a, b, slot, e)
    if string.match(guid, "^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$") == nil then return nil end
    return guid
end

local function custom_numbers(text)
    if text == nil or text == "-" then return nil end
    local out = {}
    for piece in string.gmatch(text, "[^,]+") do
        local n = tonumber(piece)
        if n == nil then return nil end
        out[#out + 1] = n
    end
    return out
end

local function custom_items()
    if io.exists(CUSTOM_SKIP_FILE) == true then
        print("[m182-custom] skipped: the previous start stopped while the copies were being created; delete " .. CUSTOM_SKIP_FILE .. " to try again")
        return
    end
    if io.exists(CUSTOM_FILE) ~= true then
        print("[m182-custom] no " .. CUSTOM_FILE .. ", no copies to create")
        return
    end
    local text = tostring(io.read_to_string(CUSTOM_FILE))
    local pools, copies = {}, {}
    for line in string.gmatch(text, "[^\r\n]+") do
        local kind, rest = string.match(line, "^(%a+)%s+(.*)$")
        if kind == "pool" then
            local base, slots = string.match(rest, "^(%d+)%s+(%d+)")
            if base ~= nil then pools[#pools + 1] = { base = tonumber(base), slots = math.min(tonumber(slots), 64) } end
        elseif kind == "copy" then
            local base, slot, d, a, lo, hi = string.match(rest, "^(%d+)%s+(%d+)%s+(%S+)%s+(%S+)%s+(%d+)%s+(%d+)")
            if base ~= nil then
                copies[#copies + 1] = { base = tonumber(base), slot = tonumber(slot), dmg = custom_numbers(d), arm = custom_numbers(a), lo = tonumber(lo), hi = tonumber(hi) }
            end
        end
    end

    if #pools == 0 and #copies == 0 then
        print("[m182-custom] " .. CUSTOM_FILE .. " is empty, no copies to create")
        return
    end

    local all = game.assets.get_resources_by_type("keen::ItemInfo")
    if type(all) ~= "table" then
        print("[m182-custom] item definitions not available")
        return
    end
    local sources, taken, existing = {}, {}, {}
    for _, resource in ipairs(all) do
        local guid = string.lower(tostring(resource.guid))
        taken[guid] = true
        existing[guid] = resource
        local id = unsigned(hash_of(resource.data.itemId))
        if id ~= nil and sources[id] == nil then sources[id] = resource end
    end
    print(string.format("[m182-custom] %d item definitions, %d pools", #all, #pools))

    local created, failed, reused = 0, 0, 0
    local made = {}
    local lines = {}
    for _, pool in ipairs(pools) do
        local base = sources[pool.base]
        if base == nil then
            print(string.format("[m182-custom] base item %d not found", pool.base))
        else
            for slot = 1, pool.slots do
                local guid = custom_guid(tostring(base.guid), slot)
                local id = custom_id(pool.base, slot)
                local key = pool.base .. ":" .. slot
                if guid == nil then
                    failed = failed + 1
                elseif taken[guid] then
                    reused = reused + 1
                    made[key] = existing[guid]
                else
                    local copy = game.assets.create_resource(base.data, "keen::ItemInfo", guid, 0)
                    local data = copy.data
                    data.itemId.value = id
                    data.objectId = guid
                    made[key] = copy
                    taken[guid] = true
                    created = created + 1
                    lines[#lines + 1] = string.format("%d,%d,%d,%s,%s", pool.base, slot, id, guid, tostring(base.data.debugName))
                end
            end
        end
    end
    print(string.format("[m182-custom] copies created=%d reused=%d failed=%d", created, reused, failed))

    local styled = 0
    for _, entry in ipairs(copies) do
        local resource = made[entry.base .. ":" .. entry.slot]
        if resource ~= nil then
            local data = resource.data
            local report = { written = 0, mismatch = 0, missing = 0 }
            if entry.dmg ~= nil and #entry.dmg == #CUSTOM_DMG_KEYS then
                local values = {}
                for index, key in ipairs(CUSTOM_DMG_KEYS) do values[key] = entry.dmg[index] end
                write_group(data.damageSetup, values, "dmg", report)
            end
            if entry.arm ~= nil and #entry.arm == #CUSTOM_ARM_KEYS then
                local values = {}
                for index, key in ipairs(CUSTOM_ARM_KEYS) do values[key] = entry.arm[index] end
                write_group(data.armorSetup, values, "arm", report)
            end
            if entry.hi > 0 then
                local span = data.itemLevelRange
                put(span, "minLevel", entry.lo)
                put(span, "maxLevel", entry.hi)
            end
            styled = styled + 1
            print(string.format("[m182-custom] copy %d/%d: wrote %d, not written %d, read-back mismatches %d", entry.base, entry.slot, report.written, report.missing, report.mismatch))
        else
            print(string.format("[m182-custom] copy %d/%d has no slot to hold it", entry.base, entry.slot))
        end
    end
    print(string.format("[m182-custom] copies styled=%d", styled))

    if loader and loader.features and loader.features.export then
        io.export("m182_custom_items.csv", "base_id,slot,item_id,guid,debug_name\n" .. table.concat(lines, "\n"))
    end
end

pcall(custom_items)
