import {
  Action,
  ActionPanel,
  Color,
  Icon,
  List,
  showHUD,
  showToast,
  Toast,
  type Image,
} from "@vicinae/api";
import { useEffect, useState } from "react";
import { defaultArgs, run, setArgs, targets, type Status, type Target } from "./waytorandr.ts";

/** Load the current CLI status on mount, exposing the result or error message and a reload. */
function useStatus() {
  const [status, setStatus] = useState<Status>();
  const [error, setError] = useState<string>();

  const reload = () =>
    run(["status", "--json"])
      .then((stdout) => setStatus(JSON.parse(stdout)))
      .catch((err: Error) => setError(err.message));

  useEffect(() => {
    reload();
  }, []);

  return { status, error, reload };
}

/** Apply a target and close with a HUD, or make it the setup default and stay open with a toast. */
async function apply(target: Target, makeDefault = false) {
  const toast = await showToast(Toast.Style.Animated, `Switching to ${target.title}`);
  try {
    await run(makeDefault ? defaultArgs(target) : setArgs(target));
    if (!makeDefault) return await showHUD(`Display layout: ${target.title}`);
    toast.style = Toast.Style.Success;
    toast.title = `Default display layout: ${target.title}`;
  } catch (err) {
    toast.style = Toast.Style.Failure;
    toast.title = makeDefault ? `Couldn't make ${target.title} the default` : `Couldn't switch to ${target.title}`;
    toast.message = (err as Error).message;
  }
}

/** Render a profile or layout with its default star, active tag, switch and make-default actions. */
function TargetItem({ target, reload }: { target: Target; reload: () => void }) {
  const icon: Image.ImageLike =
    target.kind === "profile"
      ? { source: Icon.Bookmark, tintColor: Color.PrimaryText }
      : { source: target.icon, tintColor: Color.PrimaryText };
  const accessories: List.Item.Accessory[] = [];
  if (target.isDefault) accessories.push({ icon: { source: Icon.Star, tintColor: Color.Yellow }, tooltip: "Default" });
  if (target.active) accessories.push({ tag: { value: "Active", color: Color.Green } });

  return (
    <List.Item
      title={target.title}
      subtitle={target.subtitle}
      keywords={[target.name]}
      icon={icon}
      accessories={accessories}
      actions={
        <ActionPanel>
          <Action title={`Switch to ${target.title}`} icon={Icon.Monitor} onAction={() => apply(target)} />
          {!target.isDefault && (
            <Action
              title="Switch and Make Default"
              icon={Icon.Star}
              shortcut={{ modifiers: ["ctrl"], key: "d" }}
              onAction={() => apply(target, true).then(reload)}
            />
          )}
        </ActionPanel>
      }
    />
  );
}

/** Show saved profiles and layouts, or an error when the CLI status cannot be loaded. */
export default function Layouts() {
  const { status, error, reload } = useStatus();
  const { profiles, layouts } = status ? targets(status) : { profiles: [], layouts: [] };

  return (
    <List isLoading={!status && !error} searchBarPlaceholder="Switch display layout">
      {error ? (
        <List.EmptyView icon={Icon.Monitor} title="waytorandr status failed" description={error} />
      ) : (
        <>
          <List.Section title="Saved profiles">
            {profiles.map((target) => (
              <TargetItem key={`profile:${target.name}`} target={target} reload={reload} />
            ))}
          </List.Section>
          <List.Section title="Layouts">
            {layouts.map((target) => (
              <TargetItem key={`layout:${target.name}`} target={target} reload={reload} />
            ))}
          </List.Section>
        </>
      )}
    </List>
  );
}
