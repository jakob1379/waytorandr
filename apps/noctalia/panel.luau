--!nonstrict

local layouts = require("./layouts.luau")

local targets = {}
local cursor = 1
local loading = false
local loadError = nil
local busy = false

local render

local function apply(target)
  if busy then
    return
  end
  busy = true
  noctalia.runAsync(layouts.setArgs(target), function(result)
    busy = false
    if result.exitCode == 0 then
      noctalia.notify("Display layout", target.title)
      panel.close()
    else
      noctalia.notifyError("Couldn't switch to " .. target.title, layouts.errorMessage(result))
    end
  end)
end

local function row(target, index)
  local selected = index == cursor
  local children = {
    ui.glyph({ name = target.glyph, size = 20, color = selected and "primary" or "on_surface" }),
    ui.label({ text = target.title, fontWeight = "medium" }),
    ui.label({ text = target.subtitle, color = "on_surface_variant" }),
    ui.spacer({ flexGrow = 1 }),
  }
  if target.isDefault then
    table.insert(children, ui.label({ text = "Default", fontSize = 11, color = "on_surface_variant" }))
  end
  if target.active then
    table.insert(children, ui.label({ text = "Active", fontSize = 11, fontWeight = "semibold", color = "primary" }))
  end

  return ui.row({
    key = target.kind .. ":" .. target.name,
    gap = 12,
    paddingH = 12,
    height = 42,
    align = "center",
    radius = 8,
    fill = selected and "primary/0.14" or nil,
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

local function section(title)
  return ui.label({ text = title, fontSize = 12, fontWeight = "semibold", color = "on_surface_variant" })
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
    local kind = nil
    for index, target in targets do
      if target.kind ~= kind then
        kind = target.kind
        table.insert(body, section(kind == "profile" and "Saved profiles" or "Layouts"))
      end
      table.insert(body, row(target, index))
    end
  end

  panel.render(ui.column({ flexGrow = 1, gap = 12 }, {
    ui.label({ text = "Display Layout", fontSize = 16, fontWeight = "bold" }),
    ui.scroll({ flexGrow = 1, gap = 2 }, body),
  }))
end

local function load()
  loading = true
  loadError = nil
  render()
  if not noctalia.commandExists("waytorandr") then
    loading = false
    loadError = "waytorandr is not on Noctalia's PATH"
    render()
    return
  end
  noctalia.runAsync({ "waytorandr", "status", "--json" }, function(result)
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
end

function onOpen()
  load()
end

function onKey(chord, pressed)
  if not pressed or loading or #targets == 0 then
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
