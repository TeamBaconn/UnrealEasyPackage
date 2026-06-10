import { useEffect, useRef, useState } from "react";
import { Button, Group, Modal, Stack, Text } from "@mantine/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useAppDispatch, useAppSelector } from "../store/hooks";
import { closeSettings } from "../store/uiSlice";
import { ProjectSelection } from "./gate/ProjectSelection";
import { Shell } from "./shell/Shell";
import { PluginShell } from "./shell/PluginShell";
import { SettingsModal } from "./SettingsModal";
import { activeRun, cancelBuild, onRunFinished, onRunStarted } from "../runner";
import { minimizeToTray } from "../ipc";
import { dangerSolidButton } from "../ui/buttonStyles";

/** The main window: the gate until a project is opened, then the 3-tab shell.
 *  Also owns the close guard: closing the window mid-build prompts to minimize to
 *  the tray (keep building) or discard. Closing with no build runs the default
 *  close, which exits the app (the main window owns the app lifecycle). */
export function MainSurface() {
  const dispatch = useAppDispatch();
  const screen = useAppSelector((s) => s.ui.screen);
  const mode = useAppSelector((s) => s.ui.mode);
  const settingsOpen = useAppSelector((s) => s.ui.settingsOpen);
  const buildRunning = useRef(false);
  const [prompt, setPrompt] = useState(false);

  useEffect(() => {
    // Track whether a build is in flight so the close handler can decide synchronously.
    activeRun()
      .then((r) => {
        buildRunning.current = r?.status === "running";
      })
      .catch(() => {});

    const subs: Promise<UnlistenFn>[] = [
      onRunStarted(() => {
        buildRunning.current = true;
      }),
      onRunFinished(() => {
        buildRunning.current = false;
      }),
    ];

    // Only the main window guards the close; a build running ⇒ intercept + prompt.
    // No build ⇒ let it close (→ window Destroyed → backend exits the app).
    const offClose = getCurrentWindow().onCloseRequested((event) => {
      if (!buildRunning.current) return;
      event.preventDefault();
      setPrompt(true);
    });

    return () => {
      subs.forEach((p) => void p.then((f) => f()));
      void offClose.then((f) => f());
    };
  }, []);

  async function minimize() {
    setPrompt(false);
    await minimizeToTray().catch(() => {});
  }

  async function discardAndClose() {
    setPrompt(false);
    await cancelBuild().catch(() => {});
    // destroy() (not close()) bypasses the close guard; main Destroyed → app exits.
    await getCurrentWindow().destroy();
  }

  return (
    <>
      {screen === "gate" ? <ProjectSelection /> : mode === "plugin" ? <PluginShell /> : <Shell />}
      <SettingsModal opened={settingsOpen} onClose={() => dispatch(closeSettings())} />
      <Modal
        opened={prompt}
        onClose={() => setPrompt(false)}
        title="Build in progress"
        centered
        size="md"
      >
        <Stack gap="lg">
          <Text size="sm">
            A build is still running. Minimize to the tray to keep it running in the
            background, or close the app and discard the build.
          </Text>
          <Group justify="flex-end" gap="sm">
            <Button style={dangerSolidButton()} onClick={discardAndClose}>
              Close &amp; discard build
            </Button>
            <Button onClick={minimize}>Minimize to tray</Button>
          </Group>
        </Stack>
      </Modal>
    </>
  );
}
