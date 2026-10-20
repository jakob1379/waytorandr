--!strict

local LAYOUTS = {
  { name = "horizontal", title = "Horizontal", subtitle = "Extend side by side", glyph = "layout-columns" },
  { name = "vertical", title = "Vertical", subtitle = "Extend stacked", glyph = "layout-rows" },
  { name = "mirror", title = "Mirror", subtitle = "Same picture on every screen", glyph = "copy" },
  { name = "external", title = "External", subtitle = "External screens only", glyph = "device-desktop" },
  { name = "builtin", title = "Built-in", subtitle = "Laptop screen only", glyph = "device-laptop" },
}

export type Target = {
  name: string,
  kind: string,
  title: string,
  subtitle: string,
  glyph: string,
  active: boolean,
  isDefault: boolean,
}

local function targets(status: any): { Target }
  local saved = {}
  for _, setup in status.setups or {} do
    if setup.is_current then
      saved = setup.profiles or {}
    end
  end

  local list: { Target } = {}
  local anyProfileActive = false
  for _, profile in saved do
    anyProfileActive = anyProfileActive or profile.is_active == true
    table.insert(list, {
      name = profile.name,
      kind = "profile",
      title = profile.name,
      subtitle = "",
      glyph = "bookmark",
      active = profile.is_active == true,
      isDefault = profile.is_default == true,
    })
  end
  for _, layout in LAYOUTS do
    table.insert(list, {
      name = layout.name,
      kind = "layout",
      title = layout.title,
      subtitle = layout.subtitle,
      glyph = layout.glyph,
      active = not anyProfileActive and status.layout == layout.name,
      isDefault = status.default_layout == layout.name,
    })
  end
  return list
end

local function setArgs(target: Target): { string }
  if target.kind == "profile" then
    return { "waytorandr", "set", "--profile", target.name }
  end
  return { "waytorandr", "set", target.name }
end

local function defaultArgs(target: Target): { string }
  if target.kind == "profile" then
    return { "waytorandr", "set", "--profile", target.name, "--default" }
  end
  return { "waytorandr", "set", target.name, "--default" }
end

local function errorMessage(result: any): string
  if result.timedOut then
    return "waytorandr timed out"
  end
  local stderr = string.gsub(result.stderr or "", "^Error:%s*", "")
  stderr = string.gsub(stderr, "%s+$", "")
  if stderr ~= "" then
    return stderr
  end
  return "waytorandr exited with code " .. tostring(result.exitCode)
end

return {
  targets = targets,
  setArgs = setArgs,
  defaultArgs = defaultArgs,
  errorMessage = errorMessage,
}
