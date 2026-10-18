local layouts = require("./layouts")

local function status(layout: string?)
  return {
    layout = layout,
    setups = {
      { is_current = false, profiles = { { name = "elsewhere", is_default = true, is_active = false } } },
      {
        is_current = true,
        profiles = {
          { name = "desk", is_default = true, is_active = false },
          { name = "vertical", is_default = false, is_active = false },
        },
      },
    },
  }
end

local function names(list, predicate)
  local out = {}
  for _, target in list do
    if predicate(target) then
      table.insert(out, target.kind .. ":" .. target.name)
    end
  end
  return table.concat(out, ",")
end

local list = layouts.targets(status(nil))
assert(names(list, function(t) return t.kind == "profile" end) == "profile:desk,profile:vertical")
assert(names(list, function(t) return t.kind == "layout" end) == "layout:horizontal,layout:vertical,layout:mirror,layout:external,layout:builtin")
assert(names(list, function(t) return t.isDefault end) == "profile:desk")

assert(names(layouts.targets(status("external")), function(t) return t.active end) == "layout:external")

local withActive = status("horizontal")
withActive.setups[2].profiles[1].is_active = true
assert(names(layouts.targets(withActive), function(t) return t.active end) == "profile:desk")

assert(names(layouts.targets({ setups = {} }), function(t) return t.kind == "profile" end) == "")

local shadowing = layouts.targets(status(nil))[2]
assert(table.concat(layouts.setArgs(shadowing), " ") == "waytorandr set --profile vertical")
assert(table.concat(layouts.setArgs(layouts.targets(status(nil))[3]), " ") == "waytorandr set horizontal")

assert(layouts.errorMessage({ exitCode = 1, stderr = "Error: profile 'x' not found\n" }) == "profile 'x' not found")
assert(layouts.errorMessage({ exitCode = 3, stderr = "" }) == "waytorandr exited with code 3")
assert(layouts.errorMessage({ exitCode = 9, stderr = "partial", timedOut = true }) == "waytorandr timed out")

assert(table.concat(layouts.defaultArgs(shadowing), " ") == "waytorandr set --profile vertical --default")
assert(table.concat(layouts.defaultArgs(layouts.targets(status(nil))[3]), " ") == "waytorandr set horizontal --save --default")

print("layouts: ok")
