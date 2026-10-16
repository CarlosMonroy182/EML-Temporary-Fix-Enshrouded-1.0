local lines = {}
local seen = {}

local function add(text)
    lines[#lines + 1] = text
end

local function walk(path, value, depth)
    local kind = type(value)
    add(path .. " : " .. kind)
    if depth >= 3 or seen[value] then return end
    if kind == "table" then
        seen[value] = true
        local names = {}
        pcall(function()
            for k in pairs(value) do names[#names + 1] = tostring(k) end
        end)
        table.sort(names)
        for _, k in ipairs(names) do
            local ok, child = pcall(function() return value[k] end)
            if ok then walk(path .. "." .. k, child, depth + 1) end
        end
    end
end

for _, root in ipairs({ "game", "loader", "io", "assets" }) do
    local ok, value = pcall(function() return _G[root] end)
    if ok and value ~= nil then walk(root, value, 0) end
end

local meta = {}
pcall(function()
    local mt = getmetatable(game.assets)
    if mt then
        add("game.assets metatable:")
        for k in pairs(mt) do add("  " .. tostring(k)) end
        if type(mt.__index) == "table" then
            for k in pairs(mt.__index) do add("  index." .. tostring(k)) end
        end
    end
end)

local candidates = {
    "keen::TextureResource", "keen::ui::TextureResource", "keen::UiTextureResource",
    "keen::ImageResource", "keen::SpriteResource", "keen::ui::ImageResource",
    "keen::IconResource", "keen::ecs::TemplateResource", "keen::TextResource",
}
for _, name in ipairs(candidates) do
    local ok, list = pcall(function() return game.assets.get_resources_by_type(name) end)
    local count = -1
    if ok and list ~= nil then
        pcall(function() count = #list end)
    end
    add("type " .. name .. " count=" .. tostring(count) .. " ok=" .. tostring(ok))
end

pcall(function()
    local fn = game.assets.get_resource_types or game.assets.get_types or game.assets.list_types
    if fn then
        local types = fn()
        for _, t in ipairs(types) do add("resource type: " .. tostring(t)) end
    end
end)

if loader and loader.features and loader.features.export then
    pcall(function() io.export("m182_lua_api.txt", table.concat(lines, "\n")) end)
end
print("[m182-api-probe] lines=" .. #lines)
