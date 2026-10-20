import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultArgs, setArgs, targets, type Status } from "./waytorandr.ts";

/** Create a status fixture with current and unrelated setups, applying test overrides last. */
const status = (overrides: Partial<Status>): Status => ({
  setups: [
    { is_current: false, profiles: [{ name: "elsewhere", is_default: true, is_active: false }] },
    {
      is_current: true,
      profiles: [
        { name: "desk", is_default: true, is_active: false },
        { name: "vertical", is_default: false, is_active: false },
      ],
    },
  ],
  ...overrides,
});

test("lists only profiles for the current setup", () => {
  assert.deepEqual(
    targets(status({})).profiles.map((t) => t.name),
    ["desk", "vertical"],
  );
});

test("marks the detected layout active when no saved profile is", () => {
  const { layouts } = targets(status({ layout: "vertical" }));
  assert.deepEqual(layouts.filter((t) => t.active).map((t) => t.name), ["vertical"]);
});

test("an active saved profile wins over the detected layout", () => {
  const s = status({ layout: "horizontal" });
  s.setups[1].profiles[0].is_active = true;
  const { profiles, layouts } = targets(s);
  assert.deepEqual(profiles.filter((t) => t.active).map((t) => t.name), ["desk"]);
  assert.equal(layouts.some((t) => t.active), false);
});

test("saved profiles are applied by name even when they shadow a layout", () => {
  const [, shadowing] = targets(status({})).profiles;
  assert.deepEqual(setArgs(shadowing), ["set", "--profile", "vertical"]);
  assert.deepEqual(setArgs(targets(status({})).layouts[0]), ["set", "horizontal"]);
});

test("making a target the default links layouts and selects profiles by name", () => {
  const { profiles, layouts } = targets(status({}));
  assert.deepEqual(defaultArgs(profiles[1]), ["set", "--profile", "vertical", "--default"]);
  assert.deepEqual(defaultArgs(layouts[0]), ["set", "horizontal", "--default"]);
});

test("stars the layout that is the setup default", () => {
  const s = status({ default_layout: "vertical" });
  s.setups[1].profiles[0].is_default = false;
  const { profiles, layouts } = targets(s);
  assert.deepEqual([...profiles, ...layouts].filter((t) => t.isDefault).map((t) => t.kind + ":" + t.name), ["layout:vertical"]);
});
