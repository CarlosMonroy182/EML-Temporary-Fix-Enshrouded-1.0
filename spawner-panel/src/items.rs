use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Vec2};
use serde_json::{json, Value};

use crate::theme::*;
use crate::{powers, write_atomic, App};

pub const STATS_FILE: &str = "m182_item_stats.json";
pub const MOD_DIR: &str = "zz_m182_item_stats";

pub const DMG_KEYS: [&str; 15] = [
    "speedMod",
    "dmgMod",
    "physicalCutDamage",
    "physicalPierceDamage",
    "physicalBluntDamage",
    "magicalFireDamage",
    "magicalIceDamage",
    "magicalFogDamage",
    "magicalLightningDamage",
    "healing",
    "poisonDamage",
    "woodDamage",
    "stoneDamage",
    "metalDamage",
    "explosionDamage",
];

pub const DMG_LABELS: [&str; 15] = [
    "Velocidad (×)",
    "Daño total (×)",
    "Corte",
    "Perforación",
    "Contundente",
    "Fuego",
    "Hielo",
    "Niebla",
    "Rayo",
    "Curación",
    "Veneno",
    "Madera",
    "Piedra",
    "Metal",
    "Explosión",
];

pub const ARM_KEYS: [&str; 9] = ["physical", "blunt", "pierce", "cut", "magical", "fire", "ice", "fog", "lightning"];
pub const ARM_LABELS: [&str; 9] = ["Física", "Contundente", "Perforación", "Corte", "Mágica", "Fuego", "Hielo", "Niebla", "Rayo"];

pub const RARITIES: [&str; 5] = ["Común", "Poco común", "Rara", "Épica", "Legendaria"];
pub const RARITY_NAMES: [&str; 5] = ["Common", "Uncommon", "Rare", "Epic", "Legendary"];

pub const BALANCE_LABELS: &[(&str, &str)] = &[
    ("durabilityStart", "Durabilidad base de las armas y armaduras"),
    ("durabilityGrowth", "Durabilidad extra por nivel de objeto"),
    ("durabilityLossCap", "Límite de pérdida de durabilidad"),
    ("rarityDurabilityMod.common", "Durabilidad ×  común"),
    ("rarityDurabilityMod.uncommon", "Durabilidad ×  poco común"),
    ("rarityDurabilityMod.rare", "Durabilidad ×  rara"),
    ("rarityDurabilityMod.epic", "Durabilidad ×  épica"),
    ("rarityDurabilityMod.legendary", "Durabilidad ×  legendaria"),
    ("playerBaseHealth", "Vida base del jugador"),
    ("playerHealthPerAP", "Vida por punto de atributo"),
    ("playerBaseMana", "Maná base del jugador"),
    ("playerManaPerAP", "Maná por punto de atributo"),
    ("playerBaseStamina", "Stamina base del jugador"),
    ("playerStaminaPerAP", "Stamina por punto de atributo"),
    ("critBonus", "Bonus de crítico"),
    ("baseCritChance", "Probabilidad base de crítico"),
    ("damageMod2Handed", "Multiplicador de daño a dos manos"),
    ("powerCreepWeapon", "Escalado de poder del arma"),
    ("itemLevelCap", "Nivel máximo de objeto"),
    ("playerLevelCap", "Nivel máximo del jugador"),
];

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub id: u32,
    pub guid: String,
    pub debug: String,
    pub name: String,
    pub category: String,
    pub rarity: String,
    pub stack: u32,
    pub slot: String,
    pub level_min: u32,
    pub level_max: u32,
    pub give_guid: String,
    pub pickup_guid: String,
    pub give_level: u32,
    pub dmg: [f32; 15],
    pub dmg_set: bool,
    pub arm: [f32; 9],
    pub arm_set: bool,
    pub group: String,
}

impl Item {
    pub fn rarity_index(&self) -> u32 {
        RARITY_NAMES.iter().position(|r| *r == self.rarity).unwrap_or(0) as u32
    }

    pub fn deliverable(&self) -> bool {
        !self.give_guid.is_empty() || !self.pickup_guid.is_empty()
    }

}

pub fn group_of(category: &str, slot: &str) -> &'static str {
    match category {
        "Weapons" => match slot {
            "RangedWeapon" => "Armas a distancia",
            "Shield" => "Escudos",
            _ => "Armas",
        },
        "Equipment" => {
            if slot.starts_with("Armour") {
                "Armaduras"
            } else if slot.starts_with("Vanity") {
                "Ropa cosmética"
            } else if slot.starts_with("Ring") {
                "Anillos"
            } else if slot == "Shield" {
                "Escudos"
            } else if slot == "RangedWeapon" || slot == "MainHand" {
                "Armas"
            } else {
                "Equipo y accesorios"
            }
        }
        "Tools" | "BuildTools" => "Herramientas",
        "Instrument" => "Instrumentos",
        "FishingRod" => "Pesca",
        "Consumables" => "Comida y pociones",
        "Ammunition" => "Munición",
        "Materials" => "Materiales",
        "WeaponGem" => "Gemas de arma",
        "Blueprints" => "Planos",
        "Currency" | "Collectibles" => "Coleccionables",
        "AnimalFood" | "PetFood" => "Comida de animales",
        "ColorPalette" => "Tintes",
        "Customization" => "Personalización",
        _ => "Otros",
    }
}

fn num_u(s: &str) -> u32 {
    s.trim().parse::<f64>().map(|v| v.max(0.0) as u32).unwrap_or(0)
}

fn num_f(s: &str) -> f32 {
    s.trim().parse::<f32>().unwrap_or(0.0)
}

pub fn parse_items(text: &str) -> Vec<Item> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else { return Vec::new() };
    let columns: Vec<&str> = header.trim().split(',').collect();
    let col = |name: &str| columns.iter().position(|c| *c == name);
    let c = |parts: &[&str], name: &str| -> String { col(name).and_then(|i| parts.get(i)).map(|s| s.trim().to_string()).unwrap_or_default() };
    let mut out = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 14 {
            continue;
        }
        let mut item = Item {
            id: num_u(&c(&parts, "item_id")),
            guid: c(&parts, "guid"),
            debug: c(&parts, "debug_name"),
            name: c(&parts, "name"),
            category: c(&parts, "category"),
            rarity: c(&parts, "rarity"),
            stack: num_u(&c(&parts, "max_stack")).max(1),
            slot: c(&parts, "slot"),
            level_min: num_u(&c(&parts, "level_min")).max(1),
            level_max: num_u(&c(&parts, "level_max")).max(1),
            give_guid: c(&parts, "give_guid"),
            pickup_guid: c(&parts, "pickup_guid"),
            give_level: num_u(&c(&parts, "give_level")).max(1),
            dmg_set: c(&parts, "dmg_set") == "1",
            arm_set: c(&parts, "ar_set") == "1",
            ..Default::default()
        };
        if item.id == 0 || item.guid.is_empty() {
            continue;
        }
        if item.name.is_empty() {
            item.name = item.debug.clone();
        }
        let dmg_cols = ["speed_mod", "dmg_mod", "cut", "pierce", "blunt", "fire", "ice", "fog", "lightning", "healing", "poison", "wood", "stone", "metal", "explosion"];
        for (i, name) in dmg_cols.iter().enumerate() {
            item.dmg[i] = num_f(&c(&parts, name));
        }
        let arm_cols = ["ar_physical", "ar_blunt", "ar_pierce", "ar_cut", "ar_magical", "ar_fire", "ar_ice", "ar_fog", "ar_lightning"];
        for (i, name) in arm_cols.iter().enumerate() {
            item.arm[i] = num_f(&c(&parts, name));
        }
        item.group = group_of(&item.category, &item.slot).to_string();
        out.push(item);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    out
}

pub fn load_items(dir: &Path) -> Result<Vec<Item>, String> {
    let path = dir.join("export").join("m182_items.csv");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        "La lista de ítems aún no existe. Instala la carpeta de mod m182_spawn_variants actualizada, abre el juego una vez hasta el menú principal, ciérralo y vuelve a abrir este panel.".to_string()
    })?;
    let items = parse_items(&text);
    if items.is_empty() {
        return Err("La lista de ítems está vacía. Abre el juego una vez con el mod m182_spawn_variants actualizado.".to_string());
    }
    Ok(items)
}

pub fn load_balance(dir: &Path) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("export").join("m182_balance.csv")) {
        for line in text.lines().skip(1) {
            if let Some((k, v)) = line.split_once(',') {
                if let Ok(v) = v.trim().parse::<f64>() {
                    out.insert(k.trim().to_string(), v);
                }
            }
        }
    }
    out
}

pub fn hex_of(values: &[f32]) -> String {
    values.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Edit {
    pub dmg: Option<[f32; 15]>,
    pub arm: Option<[f32; 9]>,
}

#[derive(Default)]
pub struct ItemsState {
    pub rows: Vec<Item>,
    pub balance_base: BTreeMap<String, f64>,
    pub balance: BTreeMap<String, f64>,
    pub saved: BTreeMap<u32, Edit>,
    pub working: BTreeMap<u32, Edit>,
    pub level: u32,
    pub rarity: u32,
    pub amount: u32,
    pub selected_for: Option<usize>,
    pub notice: String,
    pub outbox: VecDeque<(Instant, Value)>,
}

pub fn load_saved(dir: &Path) -> (BTreeMap<u32, Edit>, BTreeMap<String, f64>) {
    let mut items = BTreeMap::new();
    let mut balance = BTreeMap::new();
    let Some(v) = std::fs::read_to_string(dir.join(STATS_FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return (items, balance);
    };
    if let Some(map) = v.get("items").and_then(|m| m.as_object()) {
        for (k, e) in map {
            let Ok(id) = k.parse::<u32>() else { continue };
            let mut edit = Edit::default();
            if let Some(a) = e.get("dmg").and_then(|d| d.as_array()) {
                if a.len() == 15 {
                    let mut arr = [0f32; 15];
                    for (i, x) in a.iter().enumerate() {
                        arr[i] = x.as_f64().unwrap_or(0.0) as f32;
                    }
                    edit.dmg = Some(arr);
                }
            }
            if let Some(a) = e.get("arm").and_then(|d| d.as_array()) {
                if a.len() == 9 {
                    let mut arr = [0f32; 9];
                    for (i, x) in a.iter().enumerate() {
                        arr[i] = x.as_f64().unwrap_or(0.0) as f32;
                    }
                    edit.arm = Some(arr);
                }
            }
            if edit.dmg.is_some() || edit.arm.is_some() {
                items.insert(id, edit);
            }
        }
    }
    if let Some(map) = v.get("balance").and_then(|m| m.as_object()) {
        for (k, x) in map {
            if let Some(f) = x.as_f64() {
                balance.insert(k.clone(), f);
            }
        }
    }
    (items, balance)
}

pub fn stats_json(items: &BTreeMap<u32, Edit>, balance: &BTreeMap<String, f64>) -> Value {
    let mut map = serde_json::Map::new();
    for (id, e) in items {
        let mut o = serde_json::Map::new();
        if let Some(d) = e.dmg {
            o.insert("dmg".into(), json!(d.iter().map(|v| *v as f64).collect::<Vec<_>>()));
        }
        if let Some(a) = e.arm {
            o.insert("arm".into(), json!(a.iter().map(|v| *v as f64).collect::<Vec<_>>()));
        }
        map.insert(id.to_string(), Value::Object(o));
    }
    json!({"items": Value::Object(map), "balance": balance})
}

fn lua_num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{:.0}", v)
    } else {
        format!("{:.9}", v)
    }
}

pub fn lua_source(items: &BTreeMap<u32, Edit>, balance: &BTreeMap<String, f64>) -> String {
    let mut s = String::new();
    s.push_str("local OVERRIDES = {\n");
    for (id, e) in items {
        s.push_str(&format!("    [{}] = {{\n", id));
        if let Some(d) = e.dmg {
            s.push_str("        dmg = { ");
            for (i, k) in DMG_KEYS.iter().enumerate() {
                s.push_str(&format!("{} = {}, ", k, lua_num(d[i] as f64)));
            }
            s.push_str("},\n");
        }
        if let Some(a) = e.arm {
            s.push_str("        arm = { ");
            for (i, k) in ARM_KEYS.iter().enumerate() {
                s.push_str(&format!("{} = {}, ", k, lua_num(a[i] as f64)));
            }
            s.push_str("},\n");
        }
        s.push_str("    },\n");
    }
    s.push_str("}\n\nlocal BALANCE = {\n");
    for (k, v) in balance {
        s.push_str(&format!("    [\"{}\"] = {},\n", k, lua_num(*v)));
    }
    s.push_str("}\n\n");
    s.push_str(
        r#"local function try(f)
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

local function put(container, key, value)
    return pcall(function() container[key] = value end)
end

local applied, failed = 0, 0
local items = try(function() return game.assets.get_resources_by_type("keen::ItemInfo") end)
if type(items) == "table" then
    for _, resource in ipairs(items) do
        pcall(function()
            local data = resource.data
            local id = hash_of(data.itemId)
            local o = id and OVERRIDES[math.floor(id)]
            if o == nil then return end
            if o.dmg ~= nil then
                local setup = data.damageSetup
                for k, v in pairs(o.dmg) do
                    local ok
                    if k == "speedMod" or k == "dmgMod" then
                        ok = put(setup, k, v)
                    else
                        ok = put(setup.distribution, k, v)
                    end
                    if ok then applied = applied + 1 else failed = failed + 1 end
                end
            end
            if o.arm ~= nil then
                local setup = data.armorSetup
                for k, v in pairs(o.arm) do
                    if put(setup.distribution, k, v) then applied = applied + 1 else failed = failed + 1 end
                end
            end
        end)
    end
end

local tables = try(function() return game.assets.get_resources_by_type("keen::BalancingTable") end)
if type(tables) == "table" and #tables > 0 then
    local d = tables[1].data
    for key, value in pairs(BALANCE) do
        local rarity = string.match(key, "^rarityDurabilityMod%.(%a+)$")
        local ok
        if rarity then
            ok = pcall(function() d.rarityDurabilityMod[rarity] = value end)
        else
            ok = put(d, key, value)
        end
        if ok then applied = applied + 1 else failed = failed + 1 end
    end
end

print(string.format("[m182-item-stats] values applied=%d failed=%d", applied, failed))
"#,
    );
    s
}

pub fn write_mod(dir: &Path, items: &BTreeMap<u32, Edit>, balance: &BTreeMap<String, f64>) -> Result<(), String> {
    let root = dir.join("mods").join(MOD_DIR);
    let src = root.join("src");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    let manifest = r#"{
    "id": "zz_m182_item_stats",
    "name": "M182 Item Stats",
    "description": "Applies the item stat and balance values chosen in the M182 Admin Panel.",
    "version": "1.0.0",
    "capabilities": ["patch"],
    "dependencies": [],
    "authors": ["M182"],
    "license": null,
    "icon": null
}
"#;
    std::fs::write(root.join("mod.json"), manifest).map_err(|e| e.to_string())?;
    std::fs::write(src.join("mod.lua"), lua_source(items, balance)).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn live_command(item: &Item, kind: &str, old: Vec<String>, new: String) -> Value {
    json!({
        "cmd": "item_stats",
        "label": format!("{} {}", item.name, kind),
        "guid": item.guid,
        "item_id": item.id,
        "old": old,
        "new": new,
    })
}

impl ItemsState {
    pub fn base_edit(item: &Item) -> Edit {
        Edit { dmg: Some(item.dmg), arm: Some(item.arm) }
    }

    pub fn current(&self, item: &Item) -> Edit {
        self.working.get(&item.id).cloned().or_else(|| self.saved.get(&item.id).cloned()).unwrap_or_else(|| Self::base_edit(item))
    }

    pub fn is_modified(&self, item: &Item) -> bool {
        self.saved.contains_key(&item.id)
    }

    pub fn apply_item(&mut self, dir: &Path, item: &Item) -> Result<(), String> {
        let edit = self.current(item);
        let previous = self.saved.get(&item.id).cloned();
        let mut commands = Vec::new();
        if let Some(new) = edit.dmg {
            let mut old = vec![hex_of(&item.dmg)];
            if let Some(p) = previous.as_ref().and_then(|p| p.dmg) {
                old.push(hex_of(&p));
            }
            if new != item.dmg || previous.as_ref().and_then(|p| p.dmg).is_some() {
                commands.push(live_command(item, "daño", old, hex_of(&new)));
            }
        }
        if let Some(new) = edit.arm {
            let mut old = vec![hex_of(&item.arm)];
            if let Some(p) = previous.as_ref().and_then(|p| p.arm) {
                old.push(hex_of(&p));
            }
            if new != item.arm || previous.as_ref().and_then(|p| p.arm).is_some() {
                commands.push(live_command(item, "armadura", old, hex_of(&new)));
            }
        }
        let stripped = Edit {
            dmg: edit.dmg.filter(|d| *d != item.dmg),
            arm: edit.arm.filter(|a| *a != item.arm),
        };
        if stripped.dmg.is_none() && stripped.arm.is_none() {
            self.saved.remove(&item.id);
        } else {
            self.saved.insert(item.id, stripped);
        }
        self.working.remove(&item.id);
        self.persist(dir)?;
        let mut at = Instant::now();
        for c in commands {
            self.outbox.push_back((at, c));
            at += Duration::from_millis(900);
        }
        Ok(())
    }

    pub fn persist(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&stats_json(&self.saved, &self.balance_overrides())).map_err(|e| e.to_string())?;
        write_atomic(dir, STATS_FILE, "m182_item_stats.json.tmp", &text)?;
        write_mod(dir, &self.saved, &self.balance_overrides())
    }

    pub fn balance_overrides(&self) -> BTreeMap<String, f64> {
        self.balance
            .iter()
            .filter(|(k, v)| self.balance_base.get(*k).map(|b| (b - **v).abs() > 1e-9).unwrap_or(true))
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }
}

impl App {
    pub fn items_pump(&mut self) {
        while let Some((at, _)) = self.items.outbox.front() {
            if *at > Instant::now() {
                break;
            }
            let Some((_, cmd)) = self.items.outbox.pop_front() else { break };
            self.items.notice = match powers::command(&self.dir, &cmd) {
                Ok(()) => "Cambio enviado al juego. Se aplica en unos segundos y queda guardado para la próxima vez que abras el juego.".to_string(),
                Err(e) => format!("Error: {e}"),
            };
        }
    }

    pub fn item_controls(&mut self, ui: &mut egui::Ui, selected: Option<usize>) {
        let Some(index) = selected.filter(|i| *i < self.items.rows.len()) else {
            ui.label(RichText::new("Aquí puedes entregarte armas, armaduras, comida, pociones y más, con el nivel, la rareza y los stats que quieras.").color(MUTED));
            self.balance_ui(ui);
            return;
        };
        let item = self.items.rows[index].clone();
        if self.items.selected_for != Some(index) {
            self.items.selected_for = Some(index);
            self.items.level = item.give_level.clamp(1, 255);
            self.items.rarity = item.rarity_index();
            self.items.amount = 1;
        }
        let rarity_name = RARITIES[item.rarity_index().min(4) as usize];
        ui.label(RichText::new(format!("{}  ·  rareza base {}  ·  niveles {}–{}", item.group, rarity_name, item.level_min, item.level_max)).color(MUTED).size(13.0));
        let method = if !item.give_guid.is_empty() {
            ("Entrega directa disponible", GOOD)
        } else if !item.pickup_guid.is_empty() {
            ("Entrega por recogida existente", GOOD)
        } else {
            ("Este ítem no se puede entregar con este método", BAD)
        };
        ui.label(RichText::new(method.0).color(method.1).size(13.0));
        divider(ui);
        label_title(ui, "NIVEL DEL ÍTEM");
        ui.add(egui::Slider::new(&mut self.items.level, 1..=255));
        label_title(ui, "RAREZA");
        egui::ComboBox::from_id_salt("item_rarity").selected_text(RARITIES[self.items.rarity.min(4) as usize]).show_ui(ui, |ui| {
            for (i, name) in RARITIES.iter().enumerate() {
                ui.selectable_value(&mut self.items.rarity, i as u32, *name);
            }
        });
        if item.stack > 1 {
            label_title(ui, "CANTIDAD");
            let max = item.stack.clamp(2, 9999);
            self.items.amount = self.items.amount.clamp(1, max);
            ui.add(egui::Slider::new(&mut self.items.amount, 1..=max).logarithmic(true));
        } else {
            self.items.amount = 1;
        }
        divider(ui);
        self.target_controls(ui);
        divider(ui);
        if button(ui, "ENTREGAR", Vec2::new(ui.available_width(), 52.0), item.deliverable(), true) {
            self.spawn(crate::Mode::Items);
        }
        let (text, color) = self.status_text();
        if !text.is_empty() {
            ui.label(RichText::new(text).color(color));
        }
        ui.label(RichText::new("El ítem aparece en el suelo delante de ti: recógelo con la tecla de interactuar.").color(MUTED).size(13.0));
        divider(ui);
        self.item_stats_ui(ui, index);
        if !self.items.notice.is_empty() {
            ui.label(RichText::new(self.items.notice.clone()).color(GOOD));
        }
        divider(ui);
        self.balance_ui(ui);
    }

    pub fn item_stats_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let item = self.items.rows[index].clone();
        let mut edit = self.items.current(&item);
        let modified = self.items.is_modified(&item);
        label_title(ui, "MODIFICADORES DE STATS");
        if modified {
            ui.label(RichText::new("Este ítem ya tiene valores personalizados guardados.").color(GOOD));
        }
        let mut changed = false;
        let show_dmg = item.dmg_set || item.category == "Weapons" || item.dmg.iter().any(|v| *v != 0.0);
        let show_arm = item.arm_set || item.slot.starts_with("Armour") || item.arm.iter().any(|v| *v != 0.0);
        if show_dmg {
            let mut values = edit.dmg.unwrap_or(item.dmg);
            ui.label(RichText::new("Daño del arma").color(GOLD_DIM));
            egui::Grid::new(("dmg", item.id)).num_columns(2).show(ui, |ui| {
                for i in 0..15 {
                    ui.label(DMG_LABELS[i]);
                    let speed = if i < 2 { 0.01 } else { 0.5 };
                    changed |= ui.add(egui::DragValue::new(&mut values[i]).speed(speed).range(0.0..=100000.0).max_decimals(3)).changed();
                    ui.end_row();
                }
            });
            edit.dmg = Some(values);
        }
        if show_arm {
            let mut values = edit.arm.unwrap_or(item.arm);
            ui.label(RichText::new("Resistencias de la armadura").color(GOLD_DIM));
            egui::Grid::new(("arm", item.id)).num_columns(2).show(ui, |ui| {
                for i in 0..9 {
                    ui.label(ARM_LABELS[i]);
                    changed |= ui.add(egui::DragValue::new(&mut values[i]).speed(0.5).range(0.0..=100000.0).max_decimals(3)).changed();
                    ui.end_row();
                }
            });
            edit.arm = Some(values);
        }
        if !show_dmg && !show_arm {
            ui.label(RichText::new("Este ítem no tiene daño ni armadura propios. Sus modificadores son nivel, rareza y cantidad (arriba).").color(MUTED));
        }
        if changed {
            self.items.working.insert(item.id, edit.clone());
        }
        if show_dmg || show_arm {
            ui.horizontal_wrapped(|ui| {
                if button(ui, "×2", Vec2::new(54.0, 28.0), true, false) {
                    let mut e = edit.clone();
                    if let Some(d) = e.dmg.as_mut() {
                        d[1] *= 2.0;
                    }
                    if let Some(a) = e.arm.as_mut() {
                        a.iter_mut().for_each(|v| *v *= 2.0);
                    }
                    self.items.working.insert(item.id, e);
                }
                if button(ui, "×5", Vec2::new(54.0, 28.0), true, false) {
                    let mut e = edit.clone();
                    if let Some(d) = e.dmg.as_mut() {
                        d[1] *= 5.0;
                    }
                    if let Some(a) = e.arm.as_mut() {
                        a.iter_mut().for_each(|v| *v *= 5.0);
                    }
                    self.items.working.insert(item.id, e);
                }
                if button(ui, "×10", Vec2::new(58.0, 28.0), true, false) {
                    let mut e = edit.clone();
                    if let Some(d) = e.dmg.as_mut() {
                        d[1] *= 10.0;
                    }
                    if let Some(a) = e.arm.as_mut() {
                        a.iter_mut().for_each(|v| *v *= 10.0);
                    }
                    self.items.working.insert(item.id, e);
                }
            });
            ui.label(RichText::new("×2, ×5 y ×10 multiplican el daño total o toda la armadura.").color(MUTED).size(12.5));
            if button(ui, "APLICAR STATS", Vec2::new(ui.available_width(), 38.0), true, true) {
                let dir = self.dir.clone();
                let result = self.items.apply_item(&dir, &item);
                self.items.notice = match result {
                    Ok(()) => "Guardado. Enviando al juego en vivo…".to_string(),
                    Err(e) => format!("Error: {e}"),
                };
            }
            if button(ui, "Restaurar valores originales", Vec2::new(ui.available_width(), 30.0), modified || self.items.working.contains_key(&item.id), false) {
                let dir = self.dir.clone();
                let base = ItemsState::base_edit(&item);
                self.items.working.insert(item.id, base);
                let result = self.items.apply_item(&dir, &item);
                self.items.notice = match result {
                    Ok(()) => "Valores originales restaurados.".to_string(),
                    Err(e) => format!("Error: {e}"),
                };
            }
        }
    }

    pub fn balance_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(RichText::new("Ajustes globales: durabilidad, vida base…").font(title_font(14.0)).color(GOLD)).show(ui, |ui| {
            ui.label(RichText::new("Se guardan en el mod zz_m182_item_stats y se aplican al abrir el juego de nuevo.").color(MUTED).size(12.5));
            if self.items.balance.is_empty() {
                ui.label(RichText::new("Aún no hay valores. Abre el juego una vez con el mod m182_spawn_variants actualizado.").color(MUTED));
                return;
            }
            let mut changed = false;
            for (key, label) in BALANCE_LABELS {
                if let Some(value) = self.items.balance.get_mut(*key) {
                    ui.horizontal(|ui| {
                        ui.label(*label);
                        changed |= ui.add(egui::DragValue::new(value).speed(0.1).max_decimals(3)).changed();
                    });
                }
            }
            ui.horizontal(|ui| {
                if button(ui, "GUARDAR AJUSTES", Vec2::new(170.0, 32.0), true, true) {
                    let dir = self.dir.clone();
                    self.items.notice = match self.items.persist(&dir) {
                        Ok(()) => "Ajustes globales guardados. Reinicia el juego para aplicarlos.".to_string(),
                        Err(e) => format!("Error: {e}"),
                    };
                }
                if button(ui, "Restaurar", Vec2::new(110.0, 32.0), true, false) {
                    self.items.balance = self.items.balance_base.clone();
                    let dir = self.dir.clone();
                    let _ = self.items.persist(&dir);
                    self.items.notice = "Ajustes globales restaurados. Reinicia el juego.".to_string();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "item_id,guid,debug_name,name,category,rarity,max_stack,slot,level_min,level_max,give_guid,pickup_guid,pickup_name,give_level,speed_mod,dmg_mod,cut,pierce,blunt,fire,ice,fog,lightning,healing,poison,wood,stone,metal,explosion,dmg_set,ar_physical,ar_blunt,ar_pierce,ar_cut,ar_magical,ar_fire,ar_ice,ar_fog,ar_lightning,ar_set";

    fn sample() -> String {
        format!(
            "{HEADER}\n1234,g-sword,Sword_Iron,Iron Sword,Weapons,Rare,1,MainHand,3,20,give-1,,,3,1,1.25,12.5,3,0,7,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0\n\
             77,g-helm,Helm_A,Adventurer Helmet,Equipment,Common,1,Armour_Head,1,10,give-2,pick-2,LootPickup_Helm,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,5,2,3,4,0,1,0,0,0,1\n\
             9,g-bread,Bread,Bread,Consumables,Common,20,,1,1,give-3,,,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0\n"
        )
    }

    #[test]
    fn parses_items_and_groups() {
        let items = parse_items(&sample());
        assert_eq!(items.len(), 3);
        let helm = items.iter().find(|i| i.id == 77).unwrap();
        assert_eq!(helm.group, "Armaduras");
        assert_eq!(helm.arm[0], 5.0);
        assert!(helm.arm_set);
        let sword = items.iter().find(|i| i.id == 1234).unwrap();
        assert_eq!(sword.group, "Armas");
        assert_eq!(sword.dmg[1], 1.25);
        assert_eq!(sword.rarity_index(), 2);
        assert!(sword.deliverable());
        let bread = items.iter().find(|i| i.id == 9).unwrap();
        assert_eq!(bread.group, "Comida y pociones");
    }

    #[test]
    fn dump_lua_when_asked() {
        let Ok(path) = std::env::var("M182_DUMP_LUA") else { return };
        let items = parse_items(&sample());
        let sword = items.iter().find(|i| i.id == 1234).unwrap();
        let helm = items.iter().find(|i| i.id == 77).unwrap();
        let mut dmg = sword.dmg;
        dmg[1] = 5.0;
        dmg[2] = 33.5;
        let mut arm = helm.arm;
        arm[0] = 50.0;
        let mut saved = BTreeMap::new();
        saved.insert(sword.id, Edit { dmg: Some(dmg), arm: None });
        saved.insert(helm.id, Edit { dmg: None, arm: Some(arm) });
        let mut balance = BTreeMap::new();
        balance.insert("durabilityStart".to_string(), 500.0);
        balance.insert("rarityDurabilityMod.epic".to_string(), 3.5);
        std::fs::write(path, lua_source(&saved, &balance)).unwrap();
    }

    #[test]
    fn hex_roundtrip_layout() {
        assert_eq!(hex_of(&[1.0, 2.5]), "0000803f00002040");
    }

    #[test]
    fn lua_source_has_values_and_is_valid_text() {
        let items = parse_items(&sample());
        let sword = items.iter().find(|i| i.id == 1234).unwrap();
        let mut dmg = sword.dmg;
        dmg[1] = 5.0;
        let mut saved = BTreeMap::new();
        saved.insert(sword.id, Edit { dmg: Some(dmg), arm: None });
        let mut balance = BTreeMap::new();
        balance.insert("durabilityStart".to_string(), 500.0);
        balance.insert("rarityDurabilityMod.epic".to_string(), 3.5);
        let lua = lua_source(&saved, &balance);
        assert!(lua.contains("[1234] = {"));
        assert!(lua.contains("dmgMod = 5"));
        assert!(lua.contains("[\"durabilityStart\"] = 500"));
        assert!(lua.contains("[\"rarityDurabilityMod.epic\"] = 3.500000000"));
    }

    #[test]
    fn apply_item_queues_live_commands_and_writes_files() {
        let dir = std::env::temp_dir().join(format!("m182_items_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let items = parse_items(&sample());
        let sword = items.iter().find(|i| i.id == 1234).unwrap().clone();
        let mut state = ItemsState { rows: items, ..Default::default() };
        let mut edit = ItemsState::base_edit(&sword);
        edit.dmg.as_mut().unwrap()[1] = 9.0;
        state.working.insert(sword.id, edit);
        state.apply_item(&dir, &sword).unwrap();
        assert_eq!(state.outbox.len(), 1);
        let first = &state.outbox[0].1;
        assert_eq!(first["cmd"], "item_stats");
        assert_eq!(first["item_id"], 1234);
        assert_eq!(first["old"][0], hex_of(&sword.dmg));
        assert!(dir.join(STATS_FILE).exists());
        let lua = std::fs::read_to_string(dir.join("mods").join(MOD_DIR).join("src").join("mod.lua")).unwrap();
        assert!(lua.contains("dmgMod = 9"));
        let (loaded, _) = load_saved(&dir);
        assert_eq!(loaded.get(&1234).unwrap().dmg.unwrap()[1], 9.0);
        let mut edit2 = ItemsState::base_edit(&sword);
        edit2.dmg.as_mut().unwrap()[1] = 11.0;
        state.working.insert(sword.id, edit2);
        state.outbox.clear();
        state.apply_item(&dir, &sword).unwrap();
        let second = &state.outbox[0].1;
        assert_eq!(second["old"].as_array().unwrap().len(), 2);
        let restore = ItemsState::base_edit(&sword);
        state.working.insert(sword.id, restore);
        state.apply_item(&dir, &sword).unwrap();
        assert!(!state.saved.contains_key(&1234));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
