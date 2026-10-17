param([string]$GameDir = 'D:\SteamLibrary\steamapps\common\Enshrouded')

$ErrorActionPreference = 'Continue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$Base = 'https://enshrouded.wiki.gg'
$Agent = 'M182-AdminPanel/1.0 (personal modding tool; low request volume)'
$Export = Join-Path $GameDir 'export'
$Icons = Join-Path $Export 'm182_icons\wiki'
New-Item -ItemType Directory -Force -Path $Icons | Out-Null

function Get-Html([string]$path) {
    for ($try = 0; $try -lt 3; $try++) {
        try {
            $r = Invoke-WebRequest -Uri ($Base + $path) -UseBasicParsing -UserAgent $Agent -TimeoutSec 40
            Start-Sleep -Milliseconds 250
            return $r.Content
        } catch {
            Start-Sleep -Seconds 2
        }
    }
    return $null
}

function Slug([string]$s) { return ($s.ToLower() -replace '[^a-z0-9]', '') }

function Get-Category([string]$cat) {
    $pages = @()
    $subs = @()
    $path = "/wiki/Category:$cat"
    $guard = 0
    while ($path -and $guard -lt 30) {
        $guard++
        $html = Get-Html $path
        if (-not $html) { break }
        $iPages = $html.IndexOf('id="mw-pages"')
        $iSubs = $html.IndexOf('id="mw-subcategories"')
        if ($iSubs -ge 0) {
            $end = if ($iPages -gt $iSubs) { $iPages } else { $html.Length }
            $seg = $html.Substring($iSubs, $end - $iSubs)
            foreach ($m in [regex]::Matches($seg, 'href="/wiki/Category:([^"]+)"')) { $subs += $m.Groups[1].Value }
        }
        if ($iPages -ge 0) {
            $seg = $html.Substring($iPages)
            foreach ($m in [regex]::Matches($seg, '<li><a href="(/wiki/[^"]+)" title="([^"]+)"')) {
                if ($m.Groups[1].Value -notlike '/wiki/Category:*') {
                    $pages += [pscustomobject]@{ Href = $m.Groups[1].Value; Name = [System.Net.WebUtility]::HtmlDecode($m.Groups[2].Value) }
                }
            }
        }
        $next = [regex]::Match($html, '<a href="([^"]*pagefrom=[^"]*)"[^>]*>next page</a>')
        if ($next.Success) { $path = [System.Net.WebUtility]::HtmlDecode($next.Groups[1].Value) } else { $path = $null }
    }
    return [pscustomobject]@{ Pages = $pages; Subs = ($subs | Select-Object -Unique) }
}

function Save-Image([string]$href, [string]$slug) {
    $target = Join-Path $Icons ($slug + '.png')
    if (Test-Path $target) { return $true }
    $html = Get-Html $href
    if (-not $html) { return $false }
    $m = [regex]::Match($html, '<meta property="og:image" content="([^"]+)"')
    if (-not $m.Success) { return $false }
    $url = [System.Net.WebUtility]::HtmlDecode($m.Groups[1].Value)
    $file = ($url -split '\?')[0].Split('/')[-1]
    $candidates = @("$Base/images/thumb/$file/128px-$file", ($url -split '\?')[0])
    foreach ($c in $candidates) {
        try {
            Invoke-WebRequest -Uri $c -UseBasicParsing -UserAgent $Agent -TimeoutSec 40 -OutFile $target
            Start-Sleep -Milliseconds 200
            if ((Get-Item $target).Length -gt 100) { return $true }
        } catch { }
    }
    if (Test-Path $target) { Remove-Item $target -Force }
    return $false
}

Write-Host ''
Write-Host '===== M182: descargando imagenes de la wiki de Enshrouded =====' -ForegroundColor Yellow
Write-Host 'Esto tarda unos minutos la primera vez. Si lo vuelves a ejecutar, solo baja lo que falte.'
Write-Host ''

$enemyRules = @(
    @('bigfoot', 'Vukah Brawler'),
    @('giantlizard', "Hydrak'Dal"),
    @('fogger_banshee', 'Fell Banshee'),
    @('sporehead', 'Fell Sporehead'),
    @('fogger_ranger', 'Fell Ranger'),
    @('fogger_heavy_berserker', 'Fell Berserker'),
    @('black_knight', 'Fell Commander'),
    @('fogger_soldier.*armored', 'Fell Knight'),
    @('fogger_soldier|fogger_depleted', 'Fell Footsoldier'),
    @('fogger_fogcrab', 'Fell Mushroom Crab'),
    @('infestedboar', 'Fell Boar'),
    @('plant_spitter_fog', 'Fell Vicious Vine'),
    @('critterqueen', 'Fell Critter Queen'),
    @('bug_critter_suicider', 'Exploding Fell Critter'),
    @('bug_critter', 'Fell Critter'),
    @('fogger_monstrosity|fog_monstrosity', 'Fell Monstrosity'),
    @('cyclops_fog', 'Fell Cyclops'),
    @('skeleton_heavy_cyclops', 'Hollow Cyclops'),
    @('heavy_cyclops_natural', 'Cyclops'),
    @('boss_fog_dragon', 'Fell Dragon Youngling'),
    @('vulture.*fog', 'Fell Draconian Vulture'),
    @('vulture.*cold', 'Frost Draconian Vulture'),
    @('vulture', 'Draconian Vulture'),
    @('snowsabertooth', 'Snow Leopard'),
    @('sabertooth', 'Sabertooth Tiger'),
    @('skeleton.*hound', 'Hollow Hound'),
    @('skeleton.*(mage|necromancer)', 'Hollow Mage'),
    @('skeleton_heavy_hero|skeleton_elite_melee', 'Hollow Soldier'),
    @('skeleton.*(melee|weakling)', 'Hollow Skeleton'),
    @('scavenger.*acid', 'Scavenger Gorger'),
    @('scavenger.*heavy_?mage', 'Scavenger Matron'),
    @('scavenger.*tyrant', 'Scavenger Tyrant'),
    @('scavenger.*flamethrower', 'Scavenger Pyreblade'),
    @('scavenger.*bully', 'Scavenger Berserker'),
    @('scavenger.*(ranger|crossbow|cannon|repeater)', 'Scavenger Scrapgun'),
    @('scavenger.*(melee|weakling)', 'Scavenger Scalper'),
    @('vukah.*ranger', 'Vukah Ranger'),
    @('vukah.*shaman', 'Vukah Shaman'),
    @('vukah.*elite', 'Vukah Berserker'),
    @('vukah', 'Vukah Warrior'),
    @('drak0[01]', 'Drak Warrior'),
    @('drak02', 'Drak Skirmisher'),
    @('drak04', 'Drak Priest'),
    @('drak0[35]', 'Drak Chieftan'),
    @('wolf', 'Wolf'),
    @('wildbeast_rat', 'Rat'),
    @('wildbeast_bat', 'Bat'),
    @('boar', 'Boar'),
    @('scorpion', 'Scorpion'),
    @('giantcrab', 'Crab'),
    @('crocodile', 'Snapjaw'),
    @('stinger_poison', 'Pink Stinger'),
    @('stinger', 'Yellow Stinger'),
    @('icewisp', 'Frost Wisp'),
    @('willowisp', 'Wisp'),
    @('waspbee', 'Bees'),
    @('t5_goat_cold', 'Frizzy Goat'),
    @('goat', 'Common Goat'),
    @('bunny', 'Hophare'),
    @('catdeer', 'Fae Deer'),
    @('turkey', 'Flightless Redtail'),
    @('bird_desert', 'Round-Billed Sailer'),
    @('yak', 'Yak'),
    @('capybara', 'Capybara'),
    @('sanddigger', 'Dune Armadillo')
)

$rows = New-Object System.Collections.Generic.List[object]
$enemyCsv = Join-Path $Export 'm182_enemies.csv'
$wantedEnemies = @{}
if (Test-Path $enemyCsv) {
    foreach ($line in (Get-Content $enemyCsv | Select-Object -Skip 1)) {
        $p = $line.Split(',')
        if ($p.Length -lt 5 -or $p[4].Trim() -ne '') { continue }
        $lower = $p[0].ToLower()
        if ($lower -match 'base_|collider|summonstone|prop_|trap_|_old|deprecated|projectilespawner') { continue }
        foreach ($rule in $enemyRules) {
            if ($lower -match $rule[0]) {
                $rows.Add([pscustomobject]@{ Template = $p[0]; Wiki = $rule[1]; Category = 'Enemigos' })
                $wantedEnemies[$rule[1]] = $true
                break
            }
        }
    }
}
Write-Host ("Enemigos reconocidos: {0} (de {1} nombres distintos en la wiki)" -f $rows.Count, $wantedEnemies.Count)

$topCategories = @(
    @('Consumables', 'Consumibles'), @('Ammunition', 'Municion'), @('Armor', 'Armadura'), @('Collectibles', 'Coleccionables'),
    @('Dyes', 'Tintes'), @('Equipment', 'Equipo'), @('Gems', 'Gemas'), @('Resources', 'Materiales'),
    @('Tools', 'Herramientas'), @('Weapons', 'Armas'), @('Animal_Husbandry', 'Animales')
)
$items = @{}
$seen = @{}
foreach ($top in $topCategories) {
    $queue = New-Object System.Collections.Generic.Queue[object]
    $queue.Enqueue(@($top[0], 0))
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        if ($seen.ContainsKey($cur[0])) { continue }
        $seen[$cur[0]] = $true
        Write-Host ("Leyendo categoria {0}..." -f $cur[0])
        $c = Get-Category $cur[0]
        foreach ($pg in $c.Pages) {
            if (-not $items.ContainsKey($pg.Name)) { $items[$pg.Name] = [pscustomobject]@{ Href = $pg.Href; Category = $top[1] } }
        }
        if ($cur[1] -lt 2) { foreach ($s in $c.Subs) { $queue.Enqueue(@($s, $cur[1] + 1)) } }
    }
}
Write-Host ("Objetos conocidos en la wiki: {0}" -f $items.Count)

$stop = @('lootpickup','loot','node','material','consumable','ammo','equipment','collections','food','stack','small','large','big','demo','gravity','farmyield','of','the','and','a')
function Get-Tokens([string]$s) {
    $spaced = [regex]::Replace($s, '([a-z])([A-Z])', '$1 $2')
    return @(($spaced.ToLower() -split '[^a-z0-9]+') | Where-Object { $_ -and ($_ -notmatch '^\d+$') -and ($_ -notmatch '^t\d$') -and ($stop -notcontains $_) })
}
function Same-Token([string]$a, [string]$b) {
    if ($a -eq $b) { return $true }
    if ($a.Length -ge 4 -and $b.Length -ge 4) { return $a.StartsWith($b) -or $b.StartsWith($a) }
    return $false
}

$index = @{}
$wikiTokens = @{}
foreach ($name in $items.Keys) {
    $wikiTokens[$name] = Get-Tokens $name
    foreach ($t in $wikiTokens[$name]) {
        if (-not $index.ContainsKey($t)) { $index[$t] = New-Object System.Collections.Generic.List[string] }
        $index[$t].Add($name)
    }
}

$objectCsv = Join-Path $Export 'm182_objects.csv'
$wantedItems = @{}
if (Test-Path $objectCsv) {
    foreach ($line in (Get-Content $objectCsv | Select-Object -Skip 1)) {
        $name = $line.Split(',')[0]
        if ($name -notlike 'LootPickup_*' -and $name -notlike 'Loot_Node_Material_*') { continue }
        $a = Get-Tokens $name
        if ($a.Count -eq 0) { continue }
        $cands = @{}
        foreach ($t in $a) {
            foreach ($k in $index.Keys) {
                if (Same-Token $t $k) { foreach ($w in $index[$k]) { $cands[$w] = $true } }
            }
        }
        $best = $null; $bestScore = 0.0
        foreach ($w in $cands.Keys) {
            $b = $wikiTokens[$w]
            $hits = 0
            foreach ($t in $a) { foreach ($u in $b) { if (Same-Token $t $u) { $hits++; break } } }
            $score = $hits / [Math]::Max($a.Count, $b.Count)
            if ($score -gt $bestScore -or ($score -eq $bestScore -and $best -and $w.Length -lt $best.Length)) { $bestScore = $score; $best = $w }
        }
        if ($best -and $bestScore -ge 0.6) {
            $rows.Add([pscustomobject]@{ Template = $name; Wiki = $best; Category = $items[$best].Category })
            $wantedItems[$best] = $true
        }
    }
}
Write-Host ("Objetos del juego reconocidos: {0}" -f $wantedItems.Count)

$all = @{}
foreach ($k in $wantedEnemies.Keys) { $all[$k] = '/wiki/' + [Uri]::EscapeDataString($k.Replace(' ', '_')).Replace('%27', "'") }
foreach ($k in $wantedItems.Keys) { $all[$k] = $items[$k].Href }

$done = 0; $ok = 0; $total = $all.Count
$have = @{}
foreach ($k in $all.Keys) {
    $done++
    $slug = Slug $k
    if (Save-Image $all[$k] $slug) { $ok++; $have[$k] = $slug + '.png' }
    if ($done % 10 -eq 0 -or $done -eq $total) { Write-Host ("Imagenes: {0}/{1} (descargadas bien: {2})" -f $done, $total, $ok) }
}

$out = New-Object System.Collections.Generic.List[string]
$out.Add('template,wiki_name,category,file')
foreach ($r in $rows) {
    $file = ''
    if ($have.ContainsKey($r.Wiki)) { $file = $have[$r.Wiki] }
    $out.Add(('{0},{1},{2},{3}' -f $r.Template, $r.Wiki.Replace(',', ' '), $r.Category, $file))
}
[IO.File]::WriteAllLines((Join-Path $Export 'm182_wiki_index.csv'), $out, (New-Object Text.UTF8Encoding($false)))

Write-Host ''
Write-Host '===== LISTO =====' -ForegroundColor Green
Write-Host ("Imagenes guardadas en: {0}" -f $Icons)
Write-Host 'Cierra y vuelve a abrir M182_AdminPannel.exe para verlas.'
