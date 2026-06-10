import { useCallback, useEffect, useState } from "react";
import { Box, Checkbox, Stack, Text, TextInput } from "@mantine/core";
import { IconDeviceFloppy, IconShieldCheck } from "@tabler/icons-react";
import { tokens } from "../../../ui/tokens";
import { useAppSelector } from "../../../store/hooks";
import { IpcError, startResave, startValidate } from "../../../ipc";
import { activeRun, onRunFinished, onRunStarted } from "../../../runner";
import { openRunLogs } from "../../../ui/windows";
import { ToolIsland } from "./ToolIsland";
import { RemoveUepIsland } from "./RemoveUepIsland";

// The project Tools tab - editor-commandlet maintenance actions, mirroring the plugin
// shell's Tools (Actions) tab: each tool is a collapsible island with a run button that
// launches a commandlet on the shared runner and opens the streaming Run Log window.
// Reuses the same run-state plumbing (`active_run` + `uep://run-*`) so a run in flight
// disables both buttons. The `.uproject` + engine are resolved backend-side from the
// open project, so no engine picker is needed here (unlike plugin packaging).

type Tool = "resave" | "validate";

function messageOf(e: unknown): string {
  if (e instanceof IpcError) return e.appError.message;
  return e instanceof Error ? e.message : String(e);
}

export function ToolsTab() {
  const project = useAppSelector((s) => s.ui.currentProject);

  const [running, setRunning] = useState(false);
  const [activeTool, setActiveTool] = useState<Tool | null>(null);
  const [error, setError] = useState<{ tool: Tool; msg: string } | null>(null);

  // Resave options.
  const [projectOnly, setProjectOnly] = useState(true);
  const [fixup, setFixup] = useState(true);
  const [skipShader, setSkipShader] = useState(true);

  // Validate options.
  const [valSkipEngine, setValSkipEngine] = useState(true);
  const [assetType, setAssetType] = useState("");

  // Track the single shared run (build / package / tool) so both buttons disable while
  // one is in flight - the streaming log + Cancel live in the Run Log window.
  useEffect(() => {
    let alive = true;
    activeRun()
      .then((s) => alive && s && setRunning(s.status === "running"))
      .catch(() => {});
    const subs = [
      onRunStarted(() => alive && setRunning(true)),
      onRunFinished(() => alive && setRunning(false)),
    ];
    return () => {
      alive = false;
      subs.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  const runResave = useCallback(async () => {
    setError(null);
    setActiveTool("resave");
    try {
      await startResave({ projectOnly, fixupRedirectors: fixup, skipShaderCompile: skipShader });
      setRunning(true);
      await openRunLogs();
    } catch (e) {
      setError({ tool: "resave", msg: messageOf(e) });
    }
  }, [projectOnly, fixup, skipShader]);

  const runValidate = useCallback(async () => {
    setError(null);
    setActiveTool("validate");
    try {
      await startValidate({ skipEngineContent: valSkipEngine, assetType });
      setRunning(true);
      await openRunLogs();
    } catch (e) {
      setError({ tool: "validate", msg: messageOf(e) });
    }
  }, [valSkipEngine, assetType]);

  if (!project) return null;

  return (
    <Box style={{ height: "100%", overflowY: "auto" }}>
      <Stack gap="lg">
        <ToolIsland
          icon={IconDeviceFloppy}
          title="Resave Assets"
          runLabel="Resave Assets"
          runIcon={<IconDeviceFloppy size={16} />}
          running={running}
          busy={running && activeTool === "resave"}
          onRun={() => void runResave()}
        >
          <Text fz={13} c={tokens.text}>
            Load and re-save every asset in the project so the current Core Redirects bake in,
            object redirectors are fixed up, and Blueprints are re-serialized. Run this after
            renaming C++ classes or properties, or after moving assets.
          </Text>

          <Checkbox
            checked={projectOnly}
            onChange={(e) => setProjectOnly(e.currentTarget.checked)}
            label={
              <Box>
                <Text fz={13} c={tokens.text}>
                  Skip engine content
                </Text>
                <Text fz={11.5} c={tokens.textDim}>
                  Resave only this project's assets, not the engine's own content.
                </Text>
              </Box>
            }
          />
          <Checkbox
            checked={fixup}
            onChange={(e) => setFixup(e.currentTarget.checked)}
            label={
              <Box>
                <Text fz={13} c={tokens.text}>
                  Fix up redirectors
                </Text>
                <Text fz={11.5} c={tokens.textDim}>
                  Rewrite references off object redirectors, then delete the redirectors once nothing points at them.
                </Text>
              </Box>
            }
          />
          <Checkbox
            checked={skipShader}
            onChange={(e) => setSkipShader(e.currentTarget.checked)}
            label={
              <Box>
                <Text fz={13} c={tokens.text}>
                  Skip shader compilation
                </Text>
                <Text fz={11.5} c={tokens.textDim}>
                  Much faster - resaving assets doesn't need compiled shaders.
                </Text>
              </Box>
            }
          />

          {error?.tool === "resave" && (
            <Text fz={12.5} c={tokens.danger}>
              {error.msg}
            </Text>
          )}
        </ToolIsland>

        <ToolIsland
          icon={IconShieldCheck}
          title="Validate Assets"
          runLabel="Validate Assets"
          runIcon={<IconShieldCheck size={16} />}
          running={running}
          busy={running && activeTool === "validate"}
          onRun={() => void runValidate()}
        >
          <Text fz={13} c={tokens.text}>
            Run Data Validation across the project's assets: missing references, invalid property
            values, naming-convention violations, Blueprint compile errors, and any custom validators
            you've added. Errors and warnings stream to the log.
          </Text>

          <Checkbox
            checked={valSkipEngine}
            onChange={(e) => setValSkipEngine(e.currentTarget.checked)}
            label={
              <Box>
                <Text fz={13} c={tokens.text}>
                  Skip engine content
                </Text>
                <Text fz={11.5} c={tokens.textDim}>
                  Validate only this project's assets, not the engine's (engine assets rarely have validators).
                </Text>
              </Box>
            }
          />

          <Box>
            <Text fz={13} fw={600} c={tokens.text} mb={6}>
              Asset type
            </Text>
            <TextInput
              value={assetType}
              onChange={(e) => setAssetType(e.currentTarget.value)}
              placeholder="All types (e.g. StaticMesh)"
            />
            <Text fz={11} c={tokens.textDim} mt={4}>
              Leave empty to validate every type. Enter a class to validate only it and its subclasses: a
              short name (<Text span ff="monospace" fz={11}>StaticMesh</Text>) or full path
              (<Text span ff="monospace" fz={11}>/Script/Engine.StaticMesh</Text>).
            </Text>
          </Box>

          <Text fz={11.5} c={tokens.textDim}>
            Only validators enabled in the project's Data Validation settings run (C++ validators run by
            default). There's no folder filter; configure folder exclusions in those same settings.
          </Text>

          {error?.tool === "validate" && (
            <Text fz={12.5} c={tokens.danger}>
              {error.msg}
            </Text>
          )}
        </ToolIsland>

        <RemoveUepIsland running={running} />
      </Stack>
    </Box>
  );
}
