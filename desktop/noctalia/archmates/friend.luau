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
local confirmDelete = false -- ⚙ → "Delete conversation": asks before deleting

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

local function secs(s)
  s = s or 0
  return string.format("%d:%02d", s // 60, s % 60)
end

-- Borderless button with a neutral hover background. Noctalia's "ghost" button takes its hover
-- color from the palette (green in some themes), and buttons have no color option.
local hovered = {}
local function quietButton(key, opts)
  local content = {}
  if opts.glyph then
    table.insert(content, ui.glyph({ name = opts.glyph, size = opts.glyphSize or 16, color = opts.color or "on_surface" }))
  end
  if opts.text then
    table.insert(content, ui.label({ text = opts.text, fontSize = opts.fontSize or 12, color = opts.color or "on_surface" }))
  end
  return ui.row({
    key = key, gap = 6, align = "center", justify = "center", radius = 8,
    paddingH = if opts.text then 10 else 0, minWidth = 32, height = 32,
    fill = if hovered[key] then "on_surface/0.10" else "on_surface/0.0",
    tooltip = opts.tooltip,
    onClick = opts.onClick,
    onHover = function(state)
      hovered[key] = state == "true"
      render()
    end,
  }, content)
end

-- ── Voice messages ─────────────────────────────────────────────────────────────
-- The service records and plays them (its "voice desk"), and describes what it is doing in
-- ~/.cache/archmates/voice-desk.json, rewritten ten times a second while busy. The panel reads
-- that file and asks the service through `archmates voice …`: the same recorder as Winmates
-- and the universal widget (record → listen → send).
local DESK = noctalia.expandPath("~/.cache/archmates/voice-desk.json")
local desk = { state = "idle" }

local function loadDesk()
  local txt = noctalia.readFile(DESK)
  local ok, data = pcall(noctalia.json.decode, txt or "")
  if ok and data and noctalia.nowMs() - (data.ms or 0) < 5000 then desk = data else desk = { state = "idle" } end
end

local function deskIsMine()
  return desk.state ~= "idle" and desk.to == code
end

local function progressOf(id)
  local p = desk.playing
  if p and p.id == id then return math.clamp(p.position / math.max(p.secs, 0.1), 0, 1) end
  return nil
end

local function voice(args, quiet)
  noctalia.runAsync(args, function(r)
    if r.exitCode ~= 0 then
      status = "Error: " .. noctalia.string.trim(r.stderr ~= "" and r.stderr or r.stdout):gsub("^Error: ", "")
    elseif not quiet then
      status = noctalia.string.trim(r.stdout):gsub("^[✓■●▶] ", "")
    end
    loadDesk()
    loadChat()
    render()
  end, 60000)
end

-- The same waveform as in Winmates and the universal widget: bar heights come from the message
-- time, so a voice message looks the same everywhere. It fills up while playing.
local function waveform(key, seed, progress)
  local bars = {}
  for i = 0, 25 do
    local h = 5 + bit32.rshift((i * 2654435761 + seed) % 4294967296, 27) * 0.55
    local done = progress ~= nil and i / 26 < progress
    table.insert(bars, ui.box({ key = key .. "-" .. i, width = 2.5, height = h, radius = 1,
      fill = if done then "primary" else "on_surface/0.35" }))
  end
  return ui.row({ gap = 2.5, align = "center", height = 24, width = 130 }, bars)
end

local function playButton(playing, onClick)
  return ui.row({ width = 28, height = 28, radius = 14, fill = "primary", align = "center", justify = "center",
    tooltip = if playing then "Stop" else "Play", onClick = onClick }, {
    ui.glyph({ name = if playing then "player-stop-filled" else "player-play-filled", size = 13, color = "on_primary" }),
  })
end

local function voiceBubble(m)
  if not (m.file and noctalia.fileExists(m.file)) then
    return ui.label({ text = "🎤 Voice message · " .. secs(m.voice) .. " · expired", fontSize = 13, color = "on_surface_variant" })
  end
  local progress = progressOf(m.id)
  return ui.row({ gap = 8, align = "center" }, {
    playButton(progress ~= nil, function()
      if progress ~= nil then voice({ BIN, "voice", "stop-playing" }, true)
      else voice({ BIN, "voice", "play", m.file, "--id", m.id, "--no-wait" }, true) end
    end),
    waveform("wave-" .. m.id, m.ms % 4294967296, progress or 0),
    ui.label({ text = secs(m.voice), fontSize = 11, color = "on_surface_variant" }),
  })
end

-- The message field, or the recorder while recording / listening for this friend.
local function recorder()
  if desk.state == "recording" then
    local pulse = (noctalia.nowMs() // 500) % 2 == 0
    return ui.row({ gap = 6, align = "center" }, {
      quietButton("rec-cancel", { glyph = "x", tooltip = "Cancel", onClick = function() voice({ BIN, "voice", "discard" }, true) end }),
      ui.box({ width = 10, height = 10, radius = 5, fill = if pulse then "error" else "error/0.45" }),
      ui.label({ text = secs(math.floor(desk.secs or 0)) .. " / 5:00", fontFamily = "DejaVu Sans Mono", fontSize = 12, color = "on_surface" }),
      ui.column({ flexGrow = 1, gap = 4 }, {
        ui.label({ text = "", fontSize = 1 }),
        ui.progress({ progress = math.min((desk.level or 0) * 1.6, 1), fill = "primary", track = "surface_variant", height = 5, radius = 3 }),
      }),
      quietButton("rec-stop", { glyph = "player-stop-filled", tooltip = "Stop and listen", onClick = function()
        voice({ BIN, "voice", "stop" }, true)
      end }),
      ui.button({ glyph = "send", variant = "primary", onClick = function()
        status = "Sending the voice message…"
        voice({ BIN, "voice", "send-recording" })
      end }),
    })
  end
  -- Review (or sending): listen, then send or discard.
  local sending = desk.state == "sending"
  local progress = progressOf("draft")
  return ui.row({ gap = 6, align = "center" }, {
    quietButton("rec-discard", { glyph = "trash", tooltip = "Discard", onClick = function()
      if not sending then voice({ BIN, "voice", "discard" }, true) end
    end }),
    playButton(progress ~= nil, function()
      if progress ~= nil then voice({ BIN, "voice", "stop-playing" }, true)
      else voice({ BIN, "voice", "play", "--no-wait" }, true) end
    end),
    waveform("wave-draft", 0, progress or 0),
    ui.label({ text = secs(math.floor(desk.secs or 0)), fontSize = 11, color = "on_surface_variant", flexGrow = 1 }),
    ui.button({ glyph = "send", variant = "primary", onClick = function()
      if sending then return end
      status = "Sending the voice message…"
      voice({ BIN, "voice", "send-recording" })
    end }),
  })
end

local function bubble(m)
  local mine = m.dir == "out"
  local text = m.text
  if mine and not m.delivered then text = text .. "  ⏳" end
  local content = {}
  if m.voice then
    table.insert(content, voiceBubble(m))
  elseif isImage(m.file) then
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
      quietButton("decline-" .. o.id, { text = "Decline", onClick = function()
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
      quietButton("settings", { glyph = "settings", tooltip = "Delete conversation",
        onClick = function() confirmDelete = not confirmDelete; render() end }),
      quietButton("close", { glyph = "x", tooltip = "Close", onClick = function() panel.close() end }),
    }),
  }

  if confirmDelete then
    table.insert(rows, ui.column({ gap = 8, padding = 10, radius = 12, fill = "error/0.15" }, {
      ui.label({ text = "Delete your conversation with " .. friend.nickname .. " for good? Every message, voice message and file they sent you is deleted from this PC. This cannot be undone.",
        fontSize = 12, color = "on_surface", maxWidth = 330 }),
      ui.row({ gap = 8, justify = "end" }, {
        quietButton("cancel-delete", { text = "Cancel", onClick = function() confirmDelete = false; render() end }),
        ui.button({ text = "Delete", variant = "destructive", controlSize = "sm", onClick = function()
          confirmDelete = false
          run({ BIN, "delete-chat", code, "--yes" })
        end }),
      }),
    }))
  end

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

  if deskIsMine() then
    table.insert(rows, recorder())
  else
    table.insert(rows, ui.row({ gap = 6, align = "center" }, {
      quietButton("attach", { glyph = "paperclip", tooltip = "Send a file", onClick = pickAndSendFile }),
      quietButton("mic", { glyph = "microphone", tooltip = "Record a voice message (5 min max)", onClick = function()
        status = nil
        voice({ BIN, "voice", "record", code }, true)
      end }),
      ui.input({
        key = "msg-" .. inputRev, placeholder = "Message " .. friend.nickname .. "…", flexGrow = 1,
        submitOnEnter = true, focus = true,
        onChange = function(text) draft = text end,
        onSubmit = function(text) sendMessage(text) end,
      }),
      ui.button({ glyph = "send", variant = "primary", onClick = function() sendMessage(draft) end }),
    }))
  end
  if deskIsMine() and desk.error and desk.state == "review" then
    table.insert(rows, ui.label({ text = "Error: " .. desk.error, fontSize = 11, color = "error", maxWidth = 360 }))
  end
  if status then
    table.insert(rows, ui.label({ text = status, fontSize = 11, color = if status:sub(1, 6) == "Error:" then "error" else "on_surface_variant" }))
  end

  panel.render(ui.column({ gap = 12, padding = 16, flexGrow = 1 }, rows))
end

-- A notification click asks for a friend in ~/.cache/archmates/open-chat ("<code> <ms>").
local OPEN_CHAT = noctalia.expandPath("~/.cache/archmates/open-chat")

local function requestedFriend()
  local txt = noctalia.readFile(OPEN_CHAT)
  if not txt then return nil end
  noctalia.removeFile(OPEN_CHAT)
  local wanted, ms = string.match(txt, "^(%S+)%s+(%d+)")
  if wanted and os.time() * 1000 - tonumber(ms) < 30000 then return wanted end
  return nil
end

function onOpen(_context)
  code = requestedFriend() or noctalia.state.get("selected")
  status = nil
  messages = {}
  loadFriend()
  loadDesk()
  render()
  loadChat()
  panel.setWantsSecondTicks(true)
end

local ticks = 0
local lastSlow = 0
function update()
  ticks += 1
  local before = desk.state .. tostring(desk.playing ~= nil)
  loadDesk()
  local busy = desk.state == "recording" or desk.state == "sending" or desk.playing ~= nil
  -- Ten times a second while recording or playing (time, level, progress), else every second.
  noctalia.setUpdateInterval(if busy then 100 else 1000)
  if busy or before ~= desk.state .. tostring(desk.playing ~= nil) then render() end
  if noctalia.nowMs() - lastSlow >= 3000 then
    lastSlow = noctalia.nowMs()
    loadFriend()
    loadChat()
  end
end
