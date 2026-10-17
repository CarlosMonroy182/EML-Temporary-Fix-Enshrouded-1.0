local PRESETS = {
    { id = "x3",  health = 3,  damage = 1.5 },
    { id = "x10", health = 10, damage = 2.5 },
    { id = "x40", health = 40, damage = 5.0 },
}

local BASES = {
    "Enemy_LizardpeopleDrak01_Melee_Spear",
    "Enemy_LizardpeopleDrak03_Elite_Melee",
    "Enemy_LizardpeopleDrak04_Mage",
    "Enemy_Scavenger_Melee01",
    "Enemy_Scavenger_Melee_EliteArmored01",
    "Enemy_Scavenger_Ranger_Crossbow01",
    "Enemy_Scavenger_Hound",
    "Enemy_Skeleton_Melee01",
    "Enemy_Skeleton_Elite_Melee01",
    "Enemy_Skeleton_Mage01",
    "Enemy_Skeleton_Heavy_Hero",
    "Enemy_Skeleton_Elite_Necromancer",
    "Enemy_Vukah_Melee",
    "Enemy_Vukah_Melee_Elite_Armored",
    "Enemy_Vukah_Shaman_Lightning",
    "Enemy_Fogger_Soldier01",
    "Enemy_Fogger_Soldier_armored_T3_v01",
    "Enemy_Fogger_Soldier_armored_T4_v01",
    "Enemy_Fogger_Ranger01",
    "Enemy_Fogger_Depleted",
    "Enemy_Fogger_FogCrab",
    "Enemy_Fogger_JellyFloater_XL",
    "Enemy_Fog_Predator",
    "Enemy_Fogger_Banshee_Hover_Elite",
    "Enemy_Wildbeast_Wolf_Heavy",
    "Enemy_Wildbeast_Sabertooth",
    "Enemy_Wildbeast_Boar",
    "Enemy_Wildbeast_Crocodile_Amphibious",
    "Enemy_Wildbeast_Scorpion_Fire",
    "Enemy_Wildbeast_SpiderAnt_Melee",
    "Enemy_AncientConstruct_Modular04_Bull",
    "Enemy_AncientConstruct_Modular10_Slicer",
    "Enemy_AncientConstruct_Modular02_FireRoller",
}

local HOSTILE = {
    Fogger = true, Vukah = true, Scavenger = true, Wildlife = true, WildBeast = true,
    Skeleton = true, Cyclops = true, Drak = true, Ancients = true, HitAll = true, HitOthers = true,
}

local RARITIES = { Common = 0, Uncommon = 1, Rare = 2, Epic = 3, Legendary = 4 }

local function try(f, ...)
    local ok, r = pcall(f, ...)
    if ok then return r end
    return nil
end

local function str(v)
    if v == nil then return "" end
    return tostring(v)
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

local function num(v, d)
    if type(v) == "number" then return v end
    local n = tonumber(tostring(v))
    if n ~= nil then return n end
    return d or 0
end

local function safe(s)
    return (string.gsub(str(s), "[,\r\n\"]", " "))
end

local function fmt(v)
    if type(v) ~= "number" then return "0" end
    if v == math.floor(v) and math.abs(v) < 9e15 then return string.format("%.0f", v) end
    return string.format("%.9g", v)
end

local diag = {}
local function dlog(line)
    diag[#diag + 1] = line
end

local function find_comp(data, typeName)
    local found = nil
    pcall(function()
        for _, c in ipairs(data.components) do
            if str(c.type) == typeName then
                found = c
                return
            end
        end
    end)
    return found
end

local NAME_FIELDS = {
    ["keen::ecs::BossHealthBar"] = "displayName",
    ["keen::ecs::NpcSetup"] = "name",
    ["keen::ecs::LocaTagComponent"] = "locaTag",
}

local templates = try(function() return game.assets.get_resources_by_type("keen::ecs::TemplateResource") end)
local rows = {}
local byName = {}

if templates ~= nil then
    pcall(function()
        for _, res in ipairs(templates) do
            local data = res.data
            local name = str(try(function() return data.name end))
            local faction, boss, hasHealth = "", false, false
            local nameRefs = {}
            pcall(function()
                for _, c in ipairs(data.components) do
                    local ct = str(c.type)
                    if ct == "keen::ecs::Faction" then
                        faction = str(try(function() return c.value.faction end))
                    elseif ct == "keen::ecs::Health" then
                        hasHealth = true
                    elseif ct == "keen::ecs::Boss" or ct == "keen::ecs::BossHealthBar" then
                        boss = true
                    end
                    local field = NAME_FIELDS[ct]
                    if field ~= nil then
                        local ref = try(function() return c.value[field] end)
                        if ref ~= nil then nameRefs[#nameRefs + 1] = ref end
                    end
                end
            end)
            if HOSTILE[faction] and hasHealth then
                local row = { name = name, guid = str(res.guid), faction = faction, boss = boss, preset = "", res = res, nameRefs = nameRefs }
                rows[#rows + 1] = row
                if byName[name] == nil then byName[name] = row end
            end
        end
    end)
end

local objectLines = {}
local pickupByItem = {}
local genericPickup = ""
pcall(function()
    local enemyGuid = {}
    for _, r in ipairs(rows) do enemyGuid[r.guid] = true end
    for _, res in ipairs(templates) do
        local data = res.data
        local name = str(try(function() return data.name end))
        local guid = str(res.guid)
        if name == "Base_LootPickup_RandomLoot_Generic" then genericPickup = guid end
        if name ~= "" and guid ~= "" and not enemyGuid[guid] and not string.find(name, "@", 1, true) and not string.find(name, ",", 1, true) then
            local comps, count = {}, 0
            pcall(function()
                for _, c in ipairs(data.components) do
                    count = count + 1
                    local ct = (string.gsub(str(c.type), "^keen::ecs::", ""))
                    if count <= 14 then
                        comps[#comps + 1] = ct
                    end
                    if ct == "PickupItem" then
                        local id = hash_of(try(function() return c.value.itemId end))
                        if id ~= nil and id ~= 0 then
                            local old = pickupByItem[id]
                            if old == nil or (string.sub(name, 1, 11) == "LootPickup_" and string.sub(old.name, 1, 11) ~= "LootPickup_") then
                                pickupByItem[id] = { guid = guid, name = name }
                            end
                        end
                    end
                end
            end)
            objectLines[#objectLines + 1] = name .. "," .. guid .. "," .. table.concat(comps, "|")
        end
    end
    table.sort(objectLines)
end)

local function variantGuid(base, index, marker)
    local a, b, _, _, e = string.match(base, "^(%x+)%-(%x+)%-(%x+)%-(%x+)%-(%x+)$")
    if a == nil then return nil end
    return string.format("%s-%s-%s-%04x-%s", a, b, marker or "4182", index, e)
end

local created, failed = 0, 0
local errors = {}
local variants = {}

for _, baseName in ipairs(BASES) do
    local base = byName[baseName]
    if base ~= nil then
        for index, preset in ipairs(PRESETS) do
            local guid = variantGuid(base.guid, index)
            local ok, err = pcall(function()
                if guid == nil then error("bad guid") end
                local source = base.res.data
                local copy = game.assets.create_resource(source, "keen::ecs::TemplateResource", guid, 0)
                local data = copy.data
                data.name = baseName .. "@" .. preset.id
                for _, c in ipairs(data.components) do
                    local ct = str(c.type)
                    if ct == "keen::ecs::EnemyScaling" then
                        local lvl = c.value.enemyLevelScaling
                        lvl.healthScaleFactor = lvl.healthScaleFactor * preset.health
                    elseif ct == "keen::ecs::DamageMod" then
                        local ds = c.value.dataStorage
                        ds[2] = ds[2] * preset.damage
                    end
                end
            end)
            if not ok and #errors < 3 then errors[#errors + 1] = tostring(err) end
            if ok then
                created = created + 1
                variants[#variants + 1] = { name = baseName .. "@" .. preset.id, guid = guid, faction = base.faction, boss = base.boss, preset = preset.id, nameRefs = base.nameRefs }
            else
                failed = failed + 1
            end
        end
    end
end

table.sort(rows, function(a, b)
    if a.faction ~= b.faction then return a.faction < b.faction end
    return a.name < b.name
end)

-- English names of items come from the localisation table
local translations = {}

local function each(arr)
    local items = {}
    if arr == nil then return items end
    pcall(function()
        for _, v in ipairs(arr) do items[#items + 1] = v end
    end)
    if #items > 0 then return items end
    local n = try(function() return #arr end)
    if type(n) ~= "number" then n = try(function() return arr:size() end) end
    if type(n) ~= "number" then n = try(function() return arr.size end) end
    if type(n) ~= "number" then n = try(function() return arr.length end) end
    if type(n) == "number" and n > 0 then
        for i = 1, n do
            local v = try(function() return arr[i] end)
            if v == nil then v = try(function() return arr[i - 1] end) end
            if v ~= nil then items[#items + 1] = v end
        end
    end
    if #items > 0 then return items end
    pcall(function()
        for _, v in pairs(arr) do items[#items + 1] = v end
    end)
    return items
end

local function content_buffer(ref)
    local hash = try(function() return ref.hash end)
    if hash == nil then return nil, "no hash" end
    local attempts = {
        function() return game.assets.get_content(hash) end,
        function() return game.assets.get_content(tostring(hash)) end,
        function() return game.assets.get_content(ref) end,
        function() return game.assets.get_content(game.guid.from_content_hash(hash)) end,
    }
    local lastErr = "content not found"
    for index, attempt in ipairs(attempts) do
        local ok, content = pcall(attempt)
        if ok and content ~= nil then
            local ok2, buffer = pcall(function() return content:read_data() end)
            if ok2 and buffer ~= nil then return buffer, "variant " .. index end
            lastErr = "read_data: " .. tostring(buffer)
        elseif not ok then
            lastErr = tostring(content)
        end
    end
    return nil, lastErr
end

local function read_loca(ref, label)
    local buffer, how = content_buffer(ref)
    if buffer == nil then
        dlog("loca " .. label .. ": no content (" .. str(how) .. ")")
        return 0
    end
    local readers = {
        function() return buffer:read_resource("keen::LocaTagCollectionResourceData") end,
        function() return buffer:read_resource("keen::LocaTagCollectionResourceData", 0) end,
    }
    for _, reader in ipairs(readers) do
        local ok, collection = pcall(reader)
        if ok and collection ~= nil then
            local added = 0
            local tags = try(function() return collection.tags end)
            if tags ~= nil then
                local list = each(tags)
                dlog("loca " .. label .. ": tags " .. type(tags) .. ", " .. #list .. " entries")
                for _, tag in ipairs(list) do
                    local h = hash_of(try(function() return tag.id end))
                    local text = try(function() return tag.text end)
                    if h ~= nil and text ~= nil and translations[h] == nil then
                        translations[h] = tostring(text)
                        added = added + 1
                    end
                end
            end
            dlog("loca " .. label .. ": " .. added .. " texts (" .. str(how) .. ")")
            return added
        elseif not ok then
            dlog("loca " .. label .. ": read_resource failed: " .. tostring(collection))
        end
    end
    return 0
end

pcall(function()
    local locaResources = game.assets.get_resources_by_type("keen::LocaTagCollectionResource")
    local resList = each(locaResources)
    if #resList == 0 then
        dlog("loca: collection not found")
        return
    end
    dlog("loca: " .. #resList .. " collections")
    for ci, res in ipairs(resList) do
        local d = try(function() return res.data end)
        if d ~= nil then
            local eng = try(function() return d.keenglishData end)
            if eng ~= nil then read_loca(eng, ci .. "/keenglish") end
            local langs = each(try(function() return d.languages end))
            dlog("loca: " .. #langs .. " language entries")
            do
                for li, entry in ipairs(langs) do
                    local name = str(try(function() return entry.language end))
                    if li <= 3 then dlog("loca: language entry " .. name) end
                    if string.find(string.lower(name), "en", 1, true) == 1 then
                        local ref = try(function() return entry.data end)
                        if ref ~= nil then read_loca(ref, ci .. "/" .. name) end
                    end
                end
            end
        end
    end
end)
local translationCount = 0
for _ in pairs(translations) do translationCount = translationCount + 1 end
dlog("loca: " .. translationCount .. " English texts")

local function loca_text(ref)
    if ref == nil then return nil end
    local keys = {}
    local hashed = try(function() return game.guid.hash(ref) end)
    if type(hashed) == "number" then keys[#keys + 1] = hashed end
    local direct = hash_of(ref)
    if type(direct) == "number" then keys[#keys + 1] = direct end
    for _, k in ipairs(keys) do
        local text = translations[k]
        if text ~= nil and text ~= "" then return text end
    end
    return nil
end

local function display_of(row)
    for _, ref in ipairs(row.nameRefs or {}) do
        local text = loca_text(ref)
        if text ~= nil then return safe(text) end
    end
    return ""
end

local out = { "name,guid,faction,boss,preset,display" }
for _, r in ipairs(rows) do
    out[#out + 1] = string.format("%s,%s,%s,%s,%s,%s", r.name, r.guid, r.faction, tostring(r.boss), r.preset, display_of(r))
end
for _, v in ipairs(variants) do
    out[#out + 1] = string.format("%s,%s,%s,%s,%s,%s", v.name, v.guid, v.faction, tostring(v.boss), v.preset, display_of(v))
end

-- Item catalogue and delivery templates
local itemLines = {}
local itemHeader = "item_id,guid,debug_name,name,category,rarity,max_stack,slot,level_min,level_max,give_guid,pickup_guid,pickup_name,give_level,speed_mod,dmg_mod,cut,pierce,blunt,fire,ice,fog,lightning,healing,poison,wood,stone,metal,explosion,dmg_set,ar_physical,ar_blunt,ar_pierce,ar_cut,ar_magical,ar_fire,ar_ice,ar_fog,ar_lightning,ar_set"

local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
local seenItemIds = {}
if type(items) == "table" then
    for _, resource in ipairs(items) do
        pcall(function()
            local data = resource.data
            if type(data) ~= "table" and type(data) ~= "userdata" then return end
            local id = hash_of(data.itemId)
            if id == nil or id == 0 or seenItemIds[id] then return end
            seenItemIds[id] = true
            local debugName = safe(try(function() return data.debugName end))
            local category = str(try(function() return data.category end))
            local english = loca_text(data.name)
            local displayName = english or debugName
            if displayName == "" then displayName = "Item " .. tostring(id) end
            local rarity = str(try(function() return data.rarity end))
            if RARITIES[rarity] == nil then rarity = "Common" end
            local levelMin = num(try(function() return data.itemLevelRange.minLevel end), 1)
            local levelMax = num(try(function() return data.itemLevelRange.maxLevel end), 1)
            local slotName = str(try(function() return data.equipment.slot end))
            local startLevel = math.max(1, math.min(255, levelMin))

            local giveGuid = ""

            local pk = pickupByItem[id]
            local dmg = try(function() return data.damageSetup end)
            local dd = try(function() return dmg.distribution end)
            local arm = try(function() return data.armorSetup end)
            local ad = try(function() return arm.distribution end)
            local function dv(k) return fmt(num(try(function() return dd[k] end), 0)) end
            local function av(k) return fmt(num(try(function() return ad[k] end), 0)) end
            itemLines[#itemLines + 1] = table.concat({
                fmt(id), str(resource.guid), debugName, safe(displayName), category, rarity,
                fmt(num(try(function() return data.maxStackSize end), 1)), slotName, fmt(levelMin), fmt(levelMax),
                giveGuid, pk and pk.guid or "", pk and safe(pk.name) or "", fmt(startLevel),
                fmt(num(try(function() return dmg.speedMod end), 0)), fmt(num(try(function() return dmg.dmgMod end), 0)),
                dv("physicalCutDamage"), dv("physicalPierceDamage"), dv("physicalBluntDamage"), dv("magicalFireDamage"),
                dv("magicalIceDamage"), dv("magicalFogDamage"), dv("magicalLightningDamage"), dv("healing"), dv("poisonDamage"),
                dv("woodDamage"), dv("stoneDamage"), dv("metalDamage"), dv("explosionDamage"),
                tostring(try(function() return dmg.isSet end) == true and 1 or 0),
                av("physical"), av("blunt"), av("pierce"), av("cut"), av("magical"), av("fire"), av("ice"), av("fog"), av("lightning"),
                tostring(try(function() return arm.isSet end) == true and 1 or 0),
            }, ",")
        end)
    end
    table.sort(itemLines)
end
if genericPickup ~= "" then dlog("generic_pickup=" .. genericPickup) end
dlog(string.format("items: %d exported, %d English names", #itemLines, translationCount))

-- Global balance values
local balanceLines = { "key,value" }
pcall(function()
    local tables = game.assets.get_resources_by_type("keen::BalancingTable")
    if type(tables) ~= "table" or #tables == 0 then return end
    local d = tables[1].data
    local keys = {
        "playerBaseHealth", "playerHealthPerAP", "playerBaseMana", "playerManaPerAP", "playerBaseStamina", "playerStaminaPerAP",
        "critBonus", "baseCritChance", "damageMod2Handed", "durabilityStart", "durabilityGrowth", "durabilityLossCap",
        "itemLevelCap", "playerLevelCap", "powerCreepWeapon", "damageBoostFlame", "hookShotCostBase", "freezeDurationFactor",
        "maximumAnimalsInWorld", "rechargeModifier",
    }
    for _, k in ipairs(keys) do
        local v = try(function() return d[k] end)
        if type(v) == "number" then balanceLines[#balanceLines + 1] = k .. "," .. fmt(v) end
    end
    for _, r in ipairs({ "common", "uncommon", "rare", "epic", "legendary" }) do
        local v = try(function() return d.rarityDurabilityMod[r] end)
        if type(v) == "number" then balanceLines[#balanceLines + 1] = "rarityDurabilityMod." .. r .. "," .. fmt(v) end
    end
end)

if loader and loader.features and loader.features.export then
    local ok = pcall(function() io.export("m182_enemies.csv", table.concat(out, "\n")) end)
    local objectsOk = pcall(function() io.export("m182_objects.csv", "name,guid,components\n" .. table.concat(objectLines, "\n")) end)
    local itemsOk = pcall(function() io.export("m182_items.csv", itemHeader .. "\n" .. table.concat(itemLines, "\n")) end)
    pcall(function() io.export("m182_balance.csv", table.concat(balanceLines, "\n")) end)
    pcall(function()
        local body = {}
        for id, text in pairs(translations) do
            body[#body + 1] = tostring(id) .. "," .. safe(text)
        end
        table.sort(body)
        local lines = { "id,text" }
        for _, l in ipairs(body) do lines[#lines + 1] = l end
        io.export("m182_loca_en.csv", table.concat(lines, "\n"))
    end)
    pcall(function() io.export("m182_items_diag.txt", table.concat(diag, "\n")) end)
    print(string.format("[m182-spawn-variants] enemies=%d variants created=%d failed=%d export=%s objects=%d objectsExport=%s items=%d itemsExport=%s", #rows, created, failed, tostring(ok), #objectLines, tostring(objectsOk), #itemLines, tostring(itemsOk)))
    for _, e in ipairs(errors) do print("[m182-spawn-variants] error: " .. e) end
    for _, l in ipairs(diag) do print("[m182-spawn-variants] " .. l) end
else
    print("[m182-spawn-variants] export disabled")
end
