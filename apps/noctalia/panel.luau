--!nonstrict

local layouts = require("./layouts.luau")

local targets = {}
local cursor = 1
local loading = false
local loadError = nil
local busy = false
local closeButton = true
local radiusScale = 1

local render

local function apply(target, makeDefault)
  if busy then
    return
  end
  busy = true
  local args = makeDefault and layouts.defaultArgs(target) or layouts.setArgs(target)
  local started = noctalia.runAsync(args, function(result)
    busy = false
    if result.exitCode == 0 then
      noctalia.notify(makeDefault and "Default display layout" or "Display layout", target.title)
      panel.close()
    else
      noctalia.notifyError("Couldn't switch to " .. target.title, layouts.errorMessage(result))
    end
  end)
  if not started then
    busy = false
    noctalia.notifyError("Couldn't switch to " .. target.title, "waytorandr could not be started")
  end
end

local function row(target, index)
  local selected = index == cursor
  local children = {
    ui.glyph({ name = target.glyph, color = selected and "primary" or "on_surface" }),
    ui.label({ text = target.title, fontWeight = "medium" }),
    ui.label({ text = target.subtitle, color = "on_surface_variant" }),
    ui.spacer({ flexGrow = 1 }),
  }
  if target.active then
    table.insert(children, ui.glyph({ name = "check", color = "primary" }))
  end
  table.insert(children, ui.button({
    glyph = target.isDefault and "star-filled" or "star",
    variant = "ghost",
    controlSize = "sm",
    enabled = not target.isDefault,
    tooltip = target.isDefault and "Default for this setup" or "Switch to this and make it the default",
    onClick = function()
      apply(target, true)
    end,
  }))

  return ui.row({
    key = target.kind .. ":" .. target.name,
    gap = 12,
    paddingH = 12,
    height = 42,
    align = "center",
    radius = 8 * radiusScale,
    fill = selected and "primary/0.18" or "surface",
    onClick = function()
      apply(target)
    end,
    onHover = function(state)
      if state == "true" and cursor ~= index then
        cursor = index
        render()
      end
    end,
  }, children)
end

local function section(title, rows)
  return ui.column({
    gap = 6,
    padding = 12,
    radius = 12 * radiusScale,
    fill = "surface_variant",
    border = "outline/0.5",
    borderWidth = 1,
  }, {
    ui.label({ text = title, fontWeight = "bold", color = "on_surface" }),
    ui.column({ gap = 6 }, rows),
  })
end

render = function()
  local body = {}
  if loading then
    table.insert(body, ui.label({ text = "Reading displays…", color = "on_surface_variant" }))
  elseif loadError then
    table.insert(body, ui.row({ gap = 8, align = "center" }, {
      ui.glyph({ name = "alert-triangle", size = 18, color = "error" }),
      ui.label({ text = loadError, color = "error", maxLines = 3 }),
    }))
  else
    local profiles, layoutRows = {}, {}
    for index, target in targets do
      table.insert(target.kind == "profile" and profiles or layoutRows, row(target, index))
    end
    if #profiles > 0 then
      table.insert(body, section("Saved profiles", profiles))
    end
    table.insert(body, section("Layouts", layoutRows))
  end

  local header = {
    ui.label({ text = "Display Layout", fontSize = 16, fontWeight = "bold", color = "on_surface_variant", flexGrow = 1 }),
  }
  if closeButton then
    table.insert(header, ui.button({ glyph = "close", onClick = function()
      panel.close()
    end }))
  end

  panel.render(ui.column({ flexGrow = 1, gap = 12 }, {
    ui.row({ align = "center", justify = "space_between", gap = 8 }, header),
    ui.scroll({ flexGrow = 1, gap = 12 }, body),
  }))
end

local function load()
  loading = true
  loadError = nil
  targets = {}
  render()
  if not noctalia.commandExists("waytorandr") then
    loading = false
    loadError = "waytorandr is not on Noctalia's PATH"
    render()
    return
  end
  local started = noctalia.runAsync({ "waytorandr", "status", "--json" }, function(result)
    loading = false
    if result.exitCode ~= 0 then
      loadError = layouts.errorMessage(result)
    else
      local ok, status = pcall(noctalia.json.decode, result.stdout)
      if ok and type(status) == "table" then
        targets = layouts.targets(status)
        cursor = 1
        for index, target in targets do
          if target.active then
            cursor = index
          end
        end
      else
        loadError = "Couldn't read waytorandr status"
      end
    end
    render()
  end)
  if not started then
    loading = false
    loadError = "waytorandr could not be started"
    render()
  end
end

function onOpen()
  closeButton = noctalia.getSetting("shell.show_close_buttons") ~= false
  radiusScale = noctalia.getSetting("shell.corner_radius_scale") or 1
  load()
end

function onKey(chord, pressed)
  if (pressed ~= true and pressed ~= "true") or loading or #targets == 0 then
    return
  end
  if chord == "up" then
    cursor = math.max(1, cursor - 1)
    render()
  elseif chord == "down" then
    cursor = math.min(#targets, cursor + 1)
    render()
  elseif chord == "return" then
    apply(targets[cursor])
  end
end
