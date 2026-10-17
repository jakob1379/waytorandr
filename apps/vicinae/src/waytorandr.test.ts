import { test } from "node:test";
import assert from "node:assert/strict";
import { setArgs, targets, type Status } from "./waytorandr.ts";

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
