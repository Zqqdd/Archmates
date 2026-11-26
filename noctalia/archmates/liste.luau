--!nonstrict
-- Widget « Amis » : lit ~/.cache/archmates/etat.json (écrit par le service archmates) toutes les 5 s.

local MONO = "DejaVu Sans Mono"
local ETAT = noctalia.expandPath("~/.cache/archmates/etat.json")
local GLASS = { fill = "surface/0.18", radius = 18, border = "on_surface_variant/0.55", borderWidth = 1 }

local etat = nil

local function maintenantMs()
  return os.time() * 1000
end

local function depuis(ms)
  local s = math.max(0, math.floor((maintenantMs() - ms) / 1000))
  if s < 60 then return "à l'instant" end
  if s < 3600 then return string.format("il y a %d min", s // 60) end
  if s < 86400 then return string.format("il y a %d h", s // 3600) end
  return string.format("il y a %d j", s // 86400)
end

local function duree(s)
  local h, m = s // 3600, (s % 3600) // 60
  return if h > 0 then string.format("%d h %02d min", h, m) else string.format("%d min", m)
end

local function lire()
  local txt = noctalia.readFile(ETAT)
  if not txt then
    etat = nil
    return
  end
  local ok, data = pcall(noctalia.json.decode, txt)
  etat = if ok then data else nil
end

local ORDRE = { en_ligne = 0, en_attente = 1, hors_ligne = 2 }

local function avatar(a)
  if a.avatar and noctalia.fileExists(a.avatar) then
    return ui.image({ path = a.avatar, width = 36, height = 36, radius = 18, fit = "cover",
      opacity = if a.statut == "en_ligne" then 1.0 else 0.45 })
  end
  return ui.column({ width = 36, height = 36, radius = 18, fill = "surface_variant/0.6", align = "center", justify = "center" }, {
    ui.glyph({ name = "user", size = 18, color = "on_surface_variant" }),
  })
end

local function ligneAmi(a)
  local p = a.profil
  local couleur, sousTitre
  if a.statut == "en_ligne" then
    couleur = "primary"
    sousTitre = if p then (p.nom .. "@" .. p.hote .. " · " .. p.distro) else "en ligne"
  elseif a.statut == "en_attente" then
    couleur = "tertiary"
    sousTitre = "ne t'a pas encore ajouté"
  else
    couleur = "outline"
    sousTitre = if a.vu_ms then ("vu " .. depuis(a.vu_ms)) else "jamais vu"
  end
  local infobulle = nil
  if p then
    infobulle = string.format("%s@%s\n%s · noyau %s\n%s · %s · %s\nallumé depuis %s",
      p.nom, p.hote, p.distro, p.noyau, p.wm, p.terminal, p.shell, duree(p.allume_depuis_s or 0))
  end
  return ui.row({ key = a.code, gap = 10, align = "center", tooltip = infobulle }, {
    avatar(a),
    ui.column({ gap = 1, flexGrow = 1 }, {
      ui.row({ gap = 6, align = "center" }, {
        ui.box({ width = 8, height = 8, radius = 4, fill = couleur }),
        ui.label({ text = a.surnom, fontWeight = "semibold", color = "on_surface", maxLines = 1 }),
      }),
      ui.label({ text = sousTitre, fontSize = 11, color = "on_surface_variant", maxLines = 1, maxWidth = 230 }),
    }),
  })
end

function render()
  local enfants = {}
  local amis = (etat and etat.amis) or {}
  local enLigne = 0
  for _, a in amis do
    if a.statut == "en_ligne" then enLigne += 1 end
  end

  table.insert(enfants, ui.row({ gap = 8, align = "center" }, {
    ui.glyph({ name = "users", size = 15, color = "primary" }),
    ui.label({ text = "Amis", fontFamily = MONO, fontSize = 14, fontWeight = "bold", color = "primary", flexGrow = 1 }),
    ui.label({ text = string.format("%d/%d en ligne", enLigne, #amis), fontFamily = MONO, fontSize = 11, color = "on_surface_variant" }),
  }))

  if not etat then
    table.insert(enfants, ui.label({ text = "Service arrêté.\nLance : systemctl --user start archmates", fontSize = 11, color = "error" }))
  else
    if maintenantMs() - (etat.moi.maj_ms or 0) > 120000 then
      table.insert(enfants, ui.label({ text = "Service arrêté (statuts figés)", fontSize = 11, color = "error" }))
    end
    if etat.moi.invisible then
      table.insert(enfants, ui.label({ text = "Tu es en mode invisible", fontSize = 11, color = "tertiary" }))
    end
    if #amis == 0 then
      table.insert(enfants, ui.label({ text = "Aucun ami pour l'instant.\namis add <code> <nickname>", fontSize = 11, color = "on_surface_variant" }))
    else
      table.sort(amis, function(x, y)
        local ox, oy = ORDRE[x.statut] or 3, ORDRE[y.statut] or 3
        if ox ~= oy then return ox < oy end
        return string.lower(x.surnom) < string.lower(y.surnom)
      end)
      table.insert(enfants, ui.separator({}))
      for _, a in amis do
        table.insert(enfants, ligneAmi(a))
      end
    end
    table.insert(enfants, ui.row({ justify = "end" }, {
      ui.button({
        text = "Copier mon code", glyph = "copy", variant = "ghost", controlSize = "sm",
        onClick = function()
          noctalia.copyToClipboard(etat.moi.code, "text/plain;charset=utf-8")
          noctalia.notify("Amis", "Ton code d'ami est copié : envoie-le à ton ami.")
        end,
      }),
    }))
  end

  desktopWidget.render(ui.column({
    gap = 10, padding = 16, width = 300,
    fill = GLASS.fill, radius = GLASS.radius, border = GLASS.border, borderWidth = GLASS.borderWidth,
  }, enfants))
end

function update()
  noctalia.setUpdateInterval(5000)
  lire()
  render()
end

lire()
render()
