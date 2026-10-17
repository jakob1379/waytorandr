import { execFile } from "node:child_process";

export type Status = {
  profile?: string;
  layout?: string;
  setups: {
    is_current: boolean;
    profiles: { name: string; is_default: boolean; is_active: boolean }[];
  }[];
};

export type Target = {
  name: string;
  kind: "profile" | "layout";
  title: string;
  subtitle: string;
  icon: string;
  active: boolean;
  isDefault: boolean;
};

const LAYOUTS = [
  { name: "horizontal", title: "Horizontal", subtitle: "Extend side by side" },
  { name: "vertical", title: "Vertical", subtitle: "Extend stacked" },
  { name: "mirror", title: "Mirror", subtitle: "Same picture on every screen" },
  { name: "external", title: "External", subtitle: "External screens only" },
  { name: "builtin", title: "Built-in", subtitle: "Laptop screen only" },
];

export function targets(status: Status): { profiles: Target[]; layouts: Target[] } {
  const saved = status.setups.find((setup) => setup.is_current)?.profiles ?? [];
  const anyProfileActive = saved.some((profile) => profile.is_active);
  return {
    profiles: saved.map((profile) => ({
      name: profile.name,
      kind: "profile",
      title: profile.name,
      subtitle: "",
      icon: "",
      active: profile.is_active,
      isDefault: profile.is_default,
    })),
    layouts: LAYOUTS.map((layout) => ({
      ...layout,
      kind: "layout",
      icon: `${layout.name}.svg`,
      active: !anyProfileActive && status.layout === layout.name,
      isDefault: false,
    })),
  };
}

export function setArgs(target: Target): string[] {
  return target.kind === "profile" ? ["set", "--profile", target.name] : ["set", target.name];
}

export function run(args: string[]): Promise<string> {
  return new Promise((resolve, reject) =>
    execFile("waytorandr", args, (err, stdout, stderr) =>
      err ? reject(new Error(stderr.replace(/^Error:\s*/, "").trim() || err.message)) : resolve(stdout),
    ),
  );
}
