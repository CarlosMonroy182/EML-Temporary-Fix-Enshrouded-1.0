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

local function try(f, ...)
    local ok, r = pcall(f, ...)
    if ok then return r end
    return nil
end

local function str(v)
    if v == nil then return "" end
    return tostring(v)
end

local templates = try(function() return game.assets.get_resources_by_type("keen::ecs::TemplateResource") end)
local rows = {}
local byName = {}

if templates ~= nil then
    pcall(function()
        for _, res in ipairs(templates) do
            local data = res.data
            local name = str(try(function() return data.name end))
            local faction, boss, hasHealth = "", false, false
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
                end
            end)
            if HOSTILE[faction] and hasHealth then
                local row = { name = name, guid = str(res.guid), faction = faction, boss = boss, preset = "", res = res }
                rows[#rows + 1] = row
                if byName[name] == nil then byName[name] = row end
            end
        end
    end)
end

local function variantGuid(base, index)
    local a, b, _, _, e = string.match(base, "^(%x+)%-(%x+)%-(%x+)%-(%x+)%-(%x+)$")
    if a == nil then return nil end
    return string.format("%s-%s-4182-%04x-%s", a, b, index, e)
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
                variants[#variants + 1] = { name = baseName .. "@" .. preset.id, guid = guid, faction = base.faction, boss = base.boss, preset = preset.id }
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

local out = { "name,guid,faction,boss,preset" }
for _, r in ipairs(rows) do
    out[#out + 1] = string.format("%s,%s,%s,%s,%s", r.name, r.guid, r.faction, tostring(r.boss), r.preset)
end
for _, v in ipairs(variants) do
    out[#out + 1] = string.format("%s,%s,%s,%s,%s", v.name, v.guid, v.faction, tostring(v.boss), v.preset)
end

if loader and loader.features and loader.features.export then
    local ok = pcall(function() io.export("m182_enemies.csv", table.concat(out, "\n")) end)
    print(string.format("[m182-spawn-variants] enemies=%d variants created=%d failed=%d export=%s", #rows, created, failed, tostring(ok)))
    for _, e in ipairs(errors) do print("[m182-spawn-variants] error: " .. e) end
else
    print("[m182-spawn-variants] export disabled")
end
