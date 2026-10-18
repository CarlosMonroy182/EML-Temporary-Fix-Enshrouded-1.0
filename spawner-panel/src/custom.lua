local CUSTOM_FILE = "m182_custom.txt"
local CUSTOM_DMG_KEYS = { "speedMod", "dmgMod", "physicalCutDamage", "physicalPierceDamage", "physicalBluntDamage", "magicalFireDamage", "magicalIceDamage", "magicalFogDamage", "magicalLightningDamage", "healing", "poisonDamage", "woodDamage", "stoneDamage", "metalDamage", "explosionDamage" }
local CUSTOM_ARM_KEYS = { "physical", "blunt", "pierce", "cut", "magical", "fire", "ice", "fog", "lightning" }

local function custom_id(base, slot)
    return (base + slot * 2654435761 + 182) % 4294967296
end

local function custom_guid(base, slot)
    local a, b, _, _, e = string.match(tostring(base), "^(%x+)%-(%x+)%-(%x+)%-(%x+)%-(%x+)$")
    if a == nil then return nil end
    return string.format("%s-%s-5182-%04x-%s", a, b, slot, e)
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

local function custom_set_id(data, id)
    local function current() return unsigned(hash_of(try(function() return data.itemId end))) end
    local signed = id
    if signed >= 2147483648 then signed = signed - 4294967296 end
    local attempts = {
        function() data.itemId = id end,
        function() data.itemId.value = id end,
        function() data.itemId = signed end,
        function() data.itemId.value = signed end,
    }
    for _, attempt in ipairs(attempts) do
        pcall(attempt)
        if current() == id then return true end
    end
    return false
end

local function custom_items()
    local text = try(function() return io.read_to_string(CUSTOM_FILE) end)
    if text == nil then
        print("[m182-custom] no " .. CUSTOM_FILE .. ", no copies to create")
        return
    end
    text = tostring(text)
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

    local sources, taken = {}, {}
    local all = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
    if type(all) ~= "table" then
        print("[m182-custom] item definitions not available")
        return
    end
    for _, resource in ipairs(all) do
        pcall(function()
            local id = unsigned(hash_of(resource.data.itemId))
            if id ~= nil and sources[id] == nil then sources[id] = resource end
            taken[string.lower(tostring(resource.guid))] = true
        end)
    end

    local created, failed, reused = 0, 0, 0
    local made = {}
    local lines = {}
    local firstError = nil
    for _, pool in ipairs(pools) do
        local base = sources[pool.base]
        if base == nil then
            print(string.format("[m182-custom] base item %d not found", pool.base))
        else
            for slot = 1, pool.slots do
                local guid = custom_guid(tostring(base.guid), slot)
                local id = custom_id(pool.base, slot)
                local ok, err = pcall(function()
                    if guid == nil then error("bad guid") end
                    if taken[string.lower(guid)] then
                        reused = reused + 1
                        local existing = nil
                        for _, resource in ipairs(all) do
                            if string.lower(tostring(resource.guid)) == string.lower(guid) then existing = resource break end
                        end
                        if existing == nil then error("existing copy not readable") end
                        made[pool.base .. ":" .. slot] = existing
                        return
                    end
                    local copy = game.assets.create_resource(base.data, "keen::ItemInfo", guid, 0)
                    local data = copy.data
                    if not custom_set_id(data, id) then error("itemId not writable") end
                    local name = tostring(try(function() return base.data.debugName end) or "item")
                    pcall(function() data.debugName = name .. "@M182_" .. slot end)
                    made[pool.base .. ":" .. slot] = copy
                    taken[string.lower(guid)] = true
                end)
                if ok then
                    created = created + 1
                    lines[#lines + 1] = string.format("%d,%d,%d,%s,%s", pool.base, slot, id, guid, tostring(try(function() return base.data.debugName end) or ""))
                else
                    failed = failed + 1
                    if firstError == nil then firstError = tostring(err) end
                end
            end
        end
    end
    print(string.format("[m182-custom] copies created=%d reused=%d failed=%d%s", created, reused, failed, firstError and (" first error: " .. firstError) or ""))

    local styled = 0
    for _, entry in ipairs(copies) do
        local resource = made[entry.base .. ":" .. entry.slot]
        if resource ~= nil then
            pcall(function()
                local data = resource.data
                local report = { written = 0, mismatch = 0, missing = 0 }
                if entry.dmg ~= nil and #entry.dmg == #CUSTOM_DMG_KEYS then
                    local values = {}
                    for index, key in ipairs(CUSTOM_DMG_KEYS) do values[key] = entry.dmg[index] end
                    write_group(try(function() return data.damageSetup end), values, "dmg", report)
                end
                if entry.arm ~= nil and #entry.arm == #CUSTOM_ARM_KEYS then
                    local values = {}
                    for index, key in ipairs(CUSTOM_ARM_KEYS) do values[key] = entry.arm[index] end
                    write_group(try(function() return data.armorSetup end), values, "arm", report)
                end
                if entry.hi > 0 then
                    local span = try(function() return data.itemLevelRange end)
                    if span ~= nil then
                        put(span, "minLevel", entry.lo)
                        put(span, "maxLevel", entry.hi)
                    end
                end
                styled = styled + 1
                print(string.format("[m182-custom] copy %d/%d: wrote %d, not written %d, read-back mismatches %d", entry.base, entry.slot, report.written, report.missing, report.mismatch))
            end)
        else
            print(string.format("[m182-custom] copy %d/%d has no slot to hold it", entry.base, entry.slot))
        end
    end
    print(string.format("[m182-custom] copies styled=%d", styled))

    if loader and loader.features and loader.features.export then
        pcall(function() io.export("m182_custom_items.csv", "base_id,slot,item_id,guid,debug_name\n" .. table.concat(lines, "\n")) end)
    end
end

pcall(custom_items)
