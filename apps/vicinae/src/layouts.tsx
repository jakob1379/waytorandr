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
import { run, setArgs, targets, type Status, type Target } from "./waytorandr.ts";

/** Load the current CLI status once on mount, exposing the result or error message. */
function useStatus() {
  const [status, setStatus] = useState<Status>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    run(["status", "--json"])
      .then((stdout) => setStatus(JSON.parse(stdout)))
      .catch((err: Error) => setError(err.message));
  }, []);

  return { status, error };
}

/** Apply a target, showing a confirmation HUD on success or a failure toast. */
async function apply(target: Target) {
  const toast = await showToast(Toast.Style.Animated, `Switching to ${target.title}`);
  try {
    await run(setArgs(target));
    await showHUD(`Display layout: ${target.title}`);
  } catch (err) {
    toast.style = Toast.Style.Failure;
    toast.title = `Couldn't switch to ${target.title}`;
    toast.message = (err as Error).message;
  }
}

/** Render a profile or layout with its status tags and switch action. */
function TargetItem({ target }: { target: Target }) {
  const icon: Image.ImageLike =
    target.kind === "profile"
      ? { source: Icon.Bookmark, tintColor: Color.PrimaryText }
      : { source: target.icon, tintColor: Color.PrimaryText };
  const accessories: List.Item.Accessory[] = [];
  if (target.isDefault) accessories.push({ tag: "Default" });
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
        </ActionPanel>
      }
    />
  );
}

/** Show saved profiles and layouts, or an error when the CLI status cannot be loaded. */
export default function Layouts() {
  const { status, error } = useStatus();
  const { profiles, layouts } = status ? targets(status) : { profiles: [], layouts: [] };

  return (
    <List isLoading={!status && !error} searchBarPlaceholder="Switch display layout">
      {error ? (
        <List.EmptyView icon={Icon.Monitor} title="waytorandr status failed" description={error} />
      ) : (
        <>
          <List.Section title="Saved profiles">
            {profiles.map((target) => (
              <TargetItem key={`profile:${target.name}`} target={target} />
            ))}
          </List.Section>
          <List.Section title="Layouts">
            {layouts.map((target) => (
              <TargetItem key={`layout:${target.name}`} target={target} />
            ))}
          </List.Section>
        </>
      )}
    </List>
  );
}
