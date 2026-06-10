import { useCallback, useState } from "react";
import { Button, Group, Modal, Stack, Text } from "@mantine/core";
import { IconTrash } from "@tabler/icons-react";
import { tokens } from "../../../ui/tokens";
import { dangerSolidButton } from "../../../ui/buttonStyles";
import { CodeBox } from "../../../ui/CodeBox";
import { useAppDispatch, useAppSelector } from "../../../store/hooks";
import { switchProject } from "../../../store/uiSlice";
import { IpcError, removeUepData } from "../../../ipc";
import { ToolIsland } from "./ToolIsland";

function messageOf(e: unknown): string {
  if (e instanceof IpcError) return e.appError.message;
  return e instanceof Error ? e.message : String(e);
}

/** Danger island (shown in both the project and plugin Tools tabs) that deletes
 *  UnrealEasyPackage's per-project/plugin data folder (`.uep/` or `.uap/`), forgets it from
 *  Recents, and returns to the gate. Confirms first; no log window (it's an immediate file
 *  delete). The backend command resolves project-vs-plugin from managed state, so this is
 *  identical for both - it just reads the open one for display. */
export function RemoveUepIsland({ running }: { running: boolean }) {
  const dispatch = useAppDispatch();
  const project = useAppSelector((s) => s.ui.currentProject);
  const plugin = useAppSelector((s) => s.ui.currentPlugin);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dangerSolid = dangerSolidButton();

  const isPlugin = !project && !!plugin;
  const noun = isPlugin ? "plugin" : "project";
  const folder = isPlugin ? ".uap" : ".uep";
  const root = isPlugin ? plugin?.pluginRoot : project?.projectRoot;
  const stored = isPlugin
    ? "remembered engines, the output folder, and the folder name"
    : "build profiles, build history and logs, the history cache, and the saved engine override";

  const onRemove = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await removeUepData();
      setConfirm(false);
      dispatch(switchProject()); // back to the gate; the recent is already gone
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }, [dispatch]);

  if (!project && !plugin) return null;

  return (
    <>
      <ToolIsland
        icon={IconTrash}
        title="Remove UnrealEasyPackage"
        runLabel={`Remove from this ${noun}`}
        runIcon={<IconTrash size={16} />}
        running={running}
        busy={false}
        danger
        onRun={() => setConfirm(true)}
      >
        <Text fz={13} c={tokens.text}>
          Delete everything UnrealEasyPackage stores for this {noun}: the{" "}
          <Text span ff="monospace" fz={12.5}>
            {folder}
          </Text>{" "}
          folder and its contents ({stored}).
        </Text>
        <Text fz={11.5} c={tokens.textDim}>
          The {noun} is also removed from Recents and you return to the start screen. Your actual{" "}
          {noun} files (assets, source, config) are not touched.
        </Text>
      </ToolIsland>

      <Modal
        opened={confirm}
        onClose={() => {
          if (!busy) setConfirm(false);
        }}
        title="Remove UnrealEasyPackage?"
        centered
        size="md"
      >
        <Stack gap="lg">
          <Text size="sm">
            This permanently deletes UnrealEasyPackage's data for this {noun} - the{" "}
            <Text span ff="monospace" fz={12.5}>
              {folder}
            </Text>{" "}
            folder inside:
          </Text>
          {root && <CodeBox>{root}</CodeBox>}
          <Text size="sm" c={tokens.textDim}>
            Build profiles, history, and logs cannot be recovered, and the {noun} is removed from
            Recents. Your actual {noun} files are not touched. This cannot be undone.
          </Text>
          {error && (
            <Text fz={12.5} c={tokens.danger}>
              {error}
            </Text>
          )}
          <Group justify="flex-end" gap="sm">
            <Button variant="default" onClick={() => setConfirm(false)} disabled={busy}>
              Cancel
            </Button>
            <Button style={dangerSolid} onClick={() => void onRemove()} loading={busy}>
              Remove
            </Button>
          </Group>
        </Stack>
      </Modal>
    </>
  );
}
