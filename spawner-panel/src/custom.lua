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
