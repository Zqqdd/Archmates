--!nonstrict
-- "Archmates" widget: reads ~/.cache/archmates/state.json (written by the archmates service) every 5 s.

local MONO = "DejaVu Sans Mono"
local STATE = noctalia.expandPath("~/.cache/archmates/state.json")
local GLASS = { fill = "surface/0.18", radius = 18, border = "on_surface_variant/0.55", borderWidth = 1 }

local state = nil

local function nowMs()
  return os.time() * 1000
end

local function ago(ms)
  local s = math.max(0, math.floor((nowMs() - ms) / 1000))
  if s < 60 then return "just now" end
  if s < 3600 then return string.format("%d min ago", s // 60) end
  if s < 86400 then return string.format("%d h ago", s // 3600) end
  return string.format("%d d ago", s // 86400)
end

local function duration(s)
  local h, m = s // 3600, (s % 3600) // 60
  return if h > 0 then string.format("%d h %02d min", h, m) else string.format("%d min", m)
end

local function load()
  local txt = noctalia.readFile(STATE)
  if not txt then
    state = nil
    return
  end
  local ok, data = pcall(noctalia.json.decode, txt)
  state = if ok then data else nil
end

local ORDER = { online = 0, pending = 1, offline = 2 }

local function avatar(f)
  if f.avatar and noctalia.fileExists(f.avatar) then
    return ui.image({ path = f.avatar, width = 36, height = 36, radius = 18, fit = "cover",
      opacity = if f.status == "online" then 1.0 else 0.45 })
  end
  return ui.column({ width = 36, height = 36, radius = 18, fill = "surface_variant/0.6", align = "center", justify = "center" }, {
    ui.glyph({ name = "user", size = 18, color = "on_surface_variant" }),
  })
end

local function friendRow(f)
  local p = f.profile
  local color, subtitle
  if f.status == "online" then
    color = "primary"
    subtitle = if p then (p.user .. "@" .. p.host .. " · " .. p.distro) else "online"
  elseif f.status == "pending" then
    color = "tertiary"
    subtitle = "hasn't added you yet"
  else
    color = "outline"
    subtitle = if f.last_seen_ms then ("last seen " .. ago(f.last_seen_ms)) else "never seen"
  end
  local tooltip = nil
  if p then
    tooltip = string.format("%s@%s\n%s · kernel %s\n%s · %s · %s\nup %s",
      p.user, p.host, p.distro, p.kernel, p.wm, p.terminal, p.shell, duration(p.uptime_s or 0))
  end
  local unread = f.unread or 0
  return ui.row({ key = f.code, gap = 10, align = "center", tooltip = tooltip, onClick = function()
    noctalia.state.set("selected", f.code)
    -- The movable chat panel (GTK layer-shell); Noctalia's own panel if it cannot start.
    local chat = noctalia.expandPath("~/.local/bin/archmates-widget")
    if not noctalia.fileExists(chat) then chat = "/usr/bin/archmates-widget" end
    if noctalia.fileExists(chat) then
      noctalia.runAsync({ chat, "--chat-only", "--friend", f.code }, function(r)
        if r.exitCode ~= 0 then noctalia.togglePanel("archmates/archmates:friend") end
      end, 86400000)
    else
      noctalia.togglePanel("archmates/archmates:friend")
    end
  end }, {
    avatar(f),
    ui.column({ gap = 1, flexGrow = 1 }, {
      ui.row({ gap = 6, align = "center" }, {
        ui.box({ width = 8, height = 8, radius = 4, fill = color }),
        ui.label({ text = f.nickname, fontWeight = "semibold", color = "on_surface", maxLines = 1 }),
        ui.row({ visible = unread > 0, paddingH = 6, radius = 8, fill = "primary", align = "center" }, {
          ui.label({ text = tostring(unread), fontSize = 10, fontWeight = "bold", color = "on_primary" }),
        }),
      }),
      ui.label({ text = subtitle, fontSize = 11, color = "on_surface_variant", maxLines = 1, maxWidth = 230 }),
    }),
  })
end

function render()
  local children = {}
  local friends = (state and state.friends) or {}
  local online = 0
  for _, f in friends do
    if f.status == "online" then online += 1 end
  end

  table.insert(children, ui.row({ gap = 8, align = "center" }, {
    ui.glyph({ name = "users", size = 15, color = "primary" }),
    ui.label({ text = "Archmates", fontFamily = MONO, fontSize = 14, fontWeight = "bold", color = "primary", flexGrow = 1 }),
    ui.label({ text = string.format("%d/%d online", online, #friends), fontFamily = MONO, fontSize = 11, color = "on_surface_variant" }),
  }))

  if not state then
    table.insert(children, ui.label({ text = "Service stopped.\nRun: systemctl --user start archmates", fontSize = 11, color = "error" }))
  else
    if nowMs() - (state.me.updated_ms or 0) > 120000 then
      table.insert(children, ui.label({ text = "Service stopped (status frozen)", fontSize = 11, color = "error" }))
    end
    if state.me.update_available then
      table.insert(children, ui.row({ gap = 6, align = "center" }, {
        ui.glyph({ name = "download", size = 13, color = "tertiary" }),
        ui.label({ text = "Update " .. state.me.update_available .. ": archmates update --install", fontSize = 11, color = "tertiary" }),
      }))
    end
    if state.me.invisible then
      table.insert(children, ui.label({ text = "You are in invisible mode", fontSize = 11, color = "tertiary" }))
    end
    if #friends == 0 then
      table.insert(children, ui.label({ text = "No friends yet.\narchmates add <code> <nickname>", fontSize = 11, color = "on_surface_variant" }))
    else
      table.sort(friends, function(x, y)
        local ox, oy = ORDER[x.status] or 3, ORDER[y.status] or 3
        if ox ~= oy then return ox < oy end
        return string.lower(x.nickname) < string.lower(y.nickname)
      end)
      table.insert(children, ui.separator({}))
      for _, f in friends do
        table.insert(children, friendRow(f))
      end
    end
    table.insert(children, ui.row({ justify = "end" }, {
      ui.button({
        text = "Copy my code", glyph = "copy", variant = "ghost", controlSize = "sm",
        onClick = function()
          noctalia.copyToClipboard(state.me.code, "text/plain;charset=utf-8")
          noctalia.notify("Archmates", "Your friend code was copied: send it to your friend.")
        end,
      }),
    }))
  end

  desktopWidget.render(ui.column({
    gap = 10, padding = 16, width = 300,
    fill = GLASS.fill, radius = GLASS.radius, border = GLASS.border, borderWidth = GLASS.borderWidth,
  }, children))
end

function update()
  noctalia.setUpdateInterval(5000)
  load()
  render()
end

load()
render()
