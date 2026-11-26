--!nonstrict
-- Friend panel: profile, RAM / storage, conversation, send messages and files.
-- The widget stores the clicked friend's code in noctalia.state "selected".

local STATE = noctalia.expandPath("~/.cache/archmates/state.json")
local LOCAL_BIN = noctalia.expandPath("~/.local/bin/archmates")
local BIN = if noctalia.fileExists(LOCAL_BIN) then LOCAL_BIN else "archmates"

local code = nil
local friend = nil
local messages = {}
local status = nil      -- feedback line under the input ("sent", errors…)
local draft = ""
local inputRev = 0      -- a new input key clears the field after sending

local function gb(bytes)
  return string.format("%.1f", (bytes or 0) / 1073741824)
end

local function ago(ms)
  local s = math.max(0, os.time() - math.floor((ms or 0) / 1000))
  if s < 60 then return "just now" end
  if s < 3600 then return string.format("%d min ago", s // 60) end
  if s < 86400 then return string.format("%d h ago", s // 3600) end
  return string.format("%d d ago", s // 86400)
end

local function loadFriend()
  friend = nil
  local txt = noctalia.readFile(STATE)
  if not txt then return end
  local ok, data = pcall(noctalia.json.decode, txt)
  if not ok or not data then return end
  for _, f in data.friends or {} do
    -- Opened without a selection (e.g. from a keybind): show the first friend.
    if code == nil then code = f.code end
    if f.code == code then friend = f end
  end
end

local function loadChat()
  if not code then return end
  noctalia.runAsync({ BIN, "chat", code, "--json", "-n", "40" }, function(r)
    if r.exitCode == 0 then
      local ok, data = pcall(noctalia.json.decode, r.stdout)
      if ok and data then
        messages = data
        render()
      end
    end
  end, 5000)
end

local function run(args, okText)
  status = "…"
  render()
  noctalia.runAsync(args, function(r)
    if r.exitCode == 0 then
      status = okText or noctalia.string.trim(r.stdout):gsub("^✓ ", "")
    else
      status = "Error: " .. noctalia.string.trim(r.stderr ~= "" and r.stderr or r.stdout):gsub("^Error: ", "")
    end
    loadChat()
    render()
  end, 600000)
end

local function sendMessage(text)
  text = noctalia.string.trim(text or "")
  if text == "" or not code then return end
  draft = ""
  inputRev += 1
  run({ BIN, "send", code, text })
end

local function pickAndSendFile()
  -- A file chooser: zenity, kdialog, or archmates-widget's own GTK dialog.
  local pickers = {
    { "zenity", "--file-selection", "--title=Send a file" },
    { "kdialog", "--getopenfilename" },
    { noctalia.expandPath("~/.local/bin/archmates-widget"), "--pick-file" },
    { "archmates-widget", "--pick-file" },
  }
  local cmd = nil
  for _, p in pickers do
    if p[1]:sub(1, 1) == "/" and noctalia.fileExists(p[1]) or noctalia.commandExists(p[1]) then
      cmd = p
      break
    end
  end
  if not cmd then
    status = "No file chooser found: archmates send-file <friend> <file>"
    render()
    return
  end
  noctalia.runAsync(cmd, function(r)
    local path = noctalia.string.trim(r.stdout or "")
    if r.exitCode == 0 and path ~= "" then
      status = "Waiting for " .. friend.nickname .. " to accept " .. path:match("[^/]+$") .. "…"
      render()
      run({ BIN, "send-file", code, path })
    end
  end, 600000)
end

local function bar(label, used, total)
  local pct = if total and total > 0 then used / total else 0
  local color = if pct > 0.85 then "error" else "primary"
  return ui.column({ gap = 4, flexGrow = 1 }, {
    ui.row({ gap = 6 }, {
      ui.label({ text = label, fontSize = 11, color = "on_surface_variant" }),
      ui.label({ text = gb(used) .. " / " .. gb(total) .. " GB", fontSize = 11, color = "on_surface", flexGrow = 1 }),
      ui.label({ text = string.format("%d%%", math.floor(pct * 100 + 0.5)), fontSize = 11, fontWeight = "bold", color = color }),
    }),
    ui.progress({ progress = pct, fill = color, track = "surface_variant", height = 5, radius = 3 }),
  })
end

local IMAGE_EXT = { png = true, jpg = true, jpeg = true, webp = true, gif = true, bmp = true }

local function isImage(path)
  local ext = path and path:match("%.([%w]+)$")
  return ext ~= nil and IMAGE_EXT[string.lower(ext)] == true and noctalia.fileExists(path)
end

local function bubble(m)
  local mine = m.dir == "out"
  local text = m.text
  if mine and not m.delivered then text = text .. "  ⏳" end
  local content = {}
  if isImage(m.file) then
    table.insert(content, ui.image({ path = m.file, width = 240, height = 170, radius = 8, fit = "cover" }))
  else
    table.insert(content, ui.label({ text = text, fontSize = 13, color = "on_surface", maxWidth = 270 }))
  end
  table.insert(content, ui.label({ text = ago(m.ms), fontSize = 9, color = "on_surface_variant" }))
  return ui.row({ key = m.id, justify = if mine then "end" else "start" }, {
    ui.column({ gap = 4, padding = 8, radius = 12, fill = if mine then "primary/0.22" else "surface_variant/0.6" }, content),
  })
end

local function offerCard(o)
  return ui.column({ key = "offer-" .. o.id, gap = 8, padding = 10, radius = 12, fill = "tertiary/0.18" }, {
    ui.row({ gap = 8, align = "center" }, {
      ui.glyph({ name = "file-download", size = 16, color = "tertiary" }),
      ui.label({ text = string.format("%s wants to send you %s (%.1f MB)", friend.nickname, o.name, o.size / 1048576),
        fontSize = 12, color = "on_surface", maxWidth = 300 }),
    }),
    ui.row({ gap = 8, justify = "end" }, {
      ui.button({ text = "Decline", variant = "ghost", controlSize = "sm", onClick = function()
        run({ BIN, "decline", o.id })
      end }),
      ui.button({ text = "Accept", variant = "primary", controlSize = "sm", onClick = function()
        run({ BIN, "accept", o.id }, "Receiving " .. o.name .. "…")
      end }),
    }),
  })
end

function render()
  if not friend then
    panel.render(ui.column({ padding = 20 }, { ui.label({ text = "Friend not found.", color = "error" }) }))
    return
  end
  local p = friend.profile
  local online = friend.status == "online"
  local avatar = if friend.avatar and noctalia.fileExists(friend.avatar)
    then ui.image({ path = friend.avatar, width = 52, height = 52, radius = 26, fit = "cover" })
    else ui.column({ width = 52, height = 52, radius = 26, fill = "surface_variant", align = "center", justify = "center" },
      { ui.glyph({ name = "user", size = 24, color = "on_surface_variant" }) })

  local subtitle = if online then "online" elseif friend.status == "pending" then "hasn't added you yet"
    elseif friend.last_seen_ms then ("last seen " .. ago(friend.last_seen_ms)) else "offline"

  local rows = {
    ui.row({ gap = 12, align = "center" }, {
      avatar,
      ui.column({ gap = 2, flexGrow = 1 }, {
        ui.label({ text = friend.nickname, fontSize = 17, fontWeight = "bold", color = "on_surface" }),
        ui.label({ text = if p then (p.user .. "@" .. p.host .. " · " .. p.distro) else "", fontSize = 11, color = "on_surface_variant" }),
        ui.row({ gap = 6, align = "center" }, {
          ui.box({ width = 8, height = 8, radius = 4, fill = if online then "primary" else "outline" }),
          ui.label({ text = subtitle, fontSize = 11, color = "on_surface_variant" }),
        }),
      }),
      ui.button({ glyph = "x", variant = "ghost", controlSize = "sm", onClick = function() panel.close() end }),
    }),
  }

  if p and p.stats then
    table.insert(rows, ui.row({ gap = 14 }, {
      bar("RAM", p.stats.ram_used, p.stats.ram_total),
      bar("Disk", p.stats.disk_used, p.stats.disk_total),
    }))
  else
    table.insert(rows, ui.label({ text = "RAM and storage not shared", fontSize = 11, color = "on_surface_variant" }))
  end
  table.insert(rows, ui.separator({}))

  local bubbles = {}
  if #messages == 0 then
    table.insert(bubbles, ui.label({ text = "No messages yet. Say hi!", fontSize = 12, color = "on_surface_variant" }))
  end
  for _, m in messages do
    table.insert(bubbles, bubble(m))
  end
  for _, o in friend.offers or {} do
    table.insert(bubbles, offerCard(o))
  end
  table.insert(rows, ui.scroll({ flexGrow = 1, gap = 6, stickToBottom = true }, bubbles))

  table.insert(rows, ui.row({ gap = 6, align = "center" }, {
    ui.button({ glyph = "paperclip", variant = "ghost", tooltip = "Send a file", onClick = pickAndSendFile }),
    ui.input({
      key = "msg-" .. inputRev, placeholder = "Message " .. friend.nickname .. "…", flexGrow = 1,
      submitOnEnter = true, focus = true,
      onChange = function(text) draft = text end,
      onSubmit = function(text) sendMessage(text) end,
    }),
    ui.button({ glyph = "send", variant = "primary", onClick = function() sendMessage(draft) end }),
  }))
  if status then
    table.insert(rows, ui.label({ text = status, fontSize = 11, color = if status:sub(1, 6) == "Error:" then "error" else "on_surface_variant" }))
  end

  panel.render(ui.column({ gap = 12, padding = 16, flexGrow = 1 }, rows))
end

function onOpen(_context)
  code = noctalia.state.get("selected")
  status = nil
  messages = {}
  loadFriend()
  render()
  loadChat()
  panel.setWantsSecondTicks(true)
end

local ticks = 0
function update()
  ticks += 1
  if ticks % 3 == 0 then
    loadFriend()
    loadChat()
  end
end
