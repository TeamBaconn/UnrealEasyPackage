import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import {
  ActionIcon,
  Anchor,
  Box,
  Button,
  Checkbox,
  type CheckboxProps,
  Collapse,
  Divider,
  Group,
  Menu,
  Modal,
  Paper,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import {
  IconAdjustments,
  IconChevronDown,
  IconCopy,
  IconDots,
  IconFolder,
  IconInfoCircle,
  IconLock,
  IconPlus,
  IconTemplate,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { tokens } from "../../ui/tokens";
import { CodeBox } from "../../ui/CodeBox";
import { dangerSolidButton } from "../../ui/buttonStyles";
import {
  IpcError,
  createProfile,
  currentProject,
  deleteProfile,
  deleteTemplate,
  duplicateProfile,
  listProfiles,
  listTemplates,
  loadSteamSettings,
  pickDirectory,
  pickFile,
  saveProfile,
  saveSteamSettings,
  saveTemplate,
  steamOpenLoginTerminal,
  steamStatus,
  type BuildConfig,
  type SteamLocalSettings,
  type SteamStatus,
  type CleanupCategory,
  type Configuration,
  type CookMaps,
  type IncrementalCookMode,
  type DetectedProject,
  type PhaseId,
  type PhaseInfo,
  type Platform,
} from "../../ipc";
import { phaseRegistry } from "../../ipc";
import { emitProfilesChanged } from "../../ui/windows";

// A fully-populated working copy (every nested object present) so the form
// bindings never hit `undefined`; structurally a superset of `BuildConfig`, so it
// saves as-is. Defaults mirror `src-tauri/.../profiles/schema.rs`.
interface Draft {
  schemaVersion: number;
  id: string;
  name: string;
  platform: Platform;
  configs: Configuration[];
  target: string | null;
  phases: {
    build: { enabled: boolean; clean: boolean; noP4: boolean; additionalArgs: string };
    cook: {
      enabled: boolean;
      maps: CookMaps;
      cultures: string[];
      incremental: IncrementalCookMode;
      skipEditorContent: boolean;
      additionalOptions: string;
    };
    stage: {
      enabled: boolean;
      prereqs: boolean;
      forDistribution: boolean;
      debugSymbols: boolean;
      separateDebugInfo: boolean;
      additionalArgs: string;
    };
    pak: { enabled: boolean; ioStore: boolean; compressed: boolean; package: boolean; additionalArgs: string };
    archive: { enabled: boolean; additionalArgs: string };
    copyExtras: { enabled: boolean; items: { from: string; to: string }[] };
    steamUpload: {
      enabled: boolean;
      appId: string;
      description: string;
      branch: string;
      preview: boolean;
      depots: { depotId: string; path: string }[];
    };
    cleanup: { enabled: boolean; categories: CleanupCategory[]; onlyOnSuccess: boolean };
  };
  output: { baseDir: string; folderTemplate: string };
  basedOnTemplate: string | null;
  builtin: boolean;
}

const DEFAULT_FOLDER = "{project}-{platform}-{config}-{date}";

// All build configurations, for the config checkbox group.
const ALL_CONFIGS: Configuration[] = ["Debug", "DebugGame", "Development", "Test", "Shipping"];

const COMMON_CULTURES = ["en", "en-US", "de", "de-DE", "fr", "fr-FR", "es", "es-ES", "ja", "ko", "zh-Hans", "pt-BR"];

function toDraft(p: BuildConfig): Draft {
  return {
    schemaVersion: p.schemaVersion,
    id: p.id,
    name: p.name,
    platform: p.platform ?? "Win64",
    configs: p.configs?.length ? p.configs : ["Development"],
    target: p.target ?? null,
    phases: {
      build: {
        enabled: p.phases?.build?.enabled ?? true,
        clean: p.phases?.build?.clean ?? false,
        noP4: p.phases?.build?.noP4 ?? true,
        additionalArgs: p.phases?.build?.additionalArgs ?? "",
      },
      cook: {
        enabled: p.phases?.cook?.enabled ?? true,
        maps: p.phases?.cook?.maps ?? "all",
        cultures: p.phases?.cook?.cultures ?? [],
        incremental: p.phases?.cook?.incremental ?? "none",
        skipEditorContent: p.phases?.cook?.skipEditorContent ?? false,
        additionalOptions: p.phases?.cook?.additionalOptions ?? "",
      },
      stage: {
        enabled: p.phases?.stage?.enabled ?? true,
        prereqs: p.phases?.stage?.prereqs ?? true,
        forDistribution: p.phases?.stage?.forDistribution ?? false,
        debugSymbols: p.phases?.stage?.debugSymbols ?? false,
        separateDebugInfo: p.phases?.stage?.separateDebugInfo ?? false,
        additionalArgs: p.phases?.stage?.additionalArgs ?? "",
      },
      pak: {
        enabled: p.phases?.pak?.enabled ?? true,
        ioStore: p.phases?.pak?.ioStore ?? true,
        compressed: p.phases?.pak?.compressed ?? true,
        package: p.phases?.pak?.package ?? false,
        additionalArgs: p.phases?.pak?.additionalArgs ?? "",
      },
      archive: { enabled: p.phases?.archive?.enabled ?? true, additionalArgs: p.phases?.archive?.additionalArgs ?? "" },
      copyExtras: {
        enabled: p.phases?.copyExtras?.enabled ?? false,
        items: (p.phases?.copyExtras?.items ?? []).map((i) => ({ from: i.from, to: i.to ?? "." })),
      },
      steamUpload: {
        enabled: p.phases?.steamUpload?.enabled ?? false,
        appId: p.phases?.steamUpload?.appId ?? "",
        description: p.phases?.steamUpload?.description ?? "",
        branch: p.phases?.steamUpload?.branch ?? "",
        preview: p.phases?.steamUpload?.preview ?? true,
        depots: (p.phases?.steamUpload?.depots ?? []).map((d) => ({ depotId: d.depotId, path: d.path ?? "." })),
      },
      cleanup: {
        enabled: p.phases?.cleanup?.enabled ?? false,
        categories: p.phases?.cleanup?.categories ?? [],
        onlyOnSuccess: p.phases?.cleanup?.onlyOnSuccess ?? true,
      },
    },
    output: {
      baseDir: p.output?.baseDir ?? "",
      folderTemplate: p.output?.folderTemplate ?? DEFAULT_FOLDER,
    },
    basedOnTemplate: p.basedOnTemplate ?? null,
    builtin: p.builtin ?? false,
  };
}

// Locked / enabled rules - owned by the editor (UI concern), data-driven off the
// registry's `gatedBy` (shipped from Rust). `byId` maps PhaseId → its registry
// entry. A phase is locked when any of its gates isn't effectively enabled; it's
// enabled when its own `enabled` flag is on and it isn't locked. Phase config is
// indexed by id (camelCase PhaseId == the `phases` field names), so adding a phase
// needs no change here.
// Registry-configurable phases: every PhaseId except the implicit `steamLogin` preflight
// (which has no editor toggle / Draft cfg). isLocked/isEnabled/setPhaseEnabled only ever run
// on registry phases, so indexing `d.phases` by this narrower type is safe.
type ConfigPhaseId = Exclude<PhaseId, "steamLogin">;

function isLocked(id: PhaseId, d: Draft, byId: Map<PhaseId, PhaseInfo>): boolean {
  return (byId.get(id)?.gatedBy ?? []).some((g) => !isEnabled(g, d, byId));
}
function isEnabled(id: PhaseId, d: Draft, byId: Map<PhaseId, PhaseInfo>): boolean {
  return d.phases[id as ConfigPhaseId].enabled && !isLocked(id, d, byId);
}

const PLATFORM_FOLDER: Record<Platform, string> = { Win64: "Windows", Linux: "Linux", Mac: "Mac" };

function subtitleOf(p: BuildConfig): string {
  const configs = p.configs?.length ? p.configs.join(", ") : "Development";
  return `${p.platform ?? "Win64"} · ${configs} · ${p.target ?? "auto-target"}`;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

const subst = (s: string, from: string, to: string): string => s.split(from).join(to);

function renderPreview(d: Draft, project: DetectedProject | null): string {
  const now = new Date();
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  let folder = d.output.folderTemplate;
  folder = subst(folder, "{project}", project?.name ?? "project");
  folder = subst(folder, "{platform}", PLATFORM_FOLDER[d.platform]);
  // Join the configs in the backend's canonical order (schema.rs `staged_configs`:
  // the `Configuration` enum's declaration order + dedup, NOT tick order and NOT
  // lexicographic - Test precedes Shipping). ALL_CONFIGS is in that same declaration
  // order, so filtering it yields the exact path the build lands in. Doesn't touch the
  // stored `configs` ordering (the checkbox order stays as the user set it).
  folder = subst(folder, "{config}", ALL_CONFIGS.filter((c) => d.configs.includes(c)).join("+"));
  folder = subst(folder, "{profile}", d.name || "profile");
  folder = subst(folder, "{target}", d.target ?? "target");
  folder = subst(folder, "{date}", date);
  folder = subst(folder, "{time}", time);
  folder = folder.toLowerCase();
  const baseRaw = d.output.baseDir.trim();
  const isAbsolute = isAbsolutePath(baseRaw); // absolute/direct vs project-local
  const root = (project?.projectRoot ?? "").replace(/[\\/]+$/, "");
  const localPart = baseRaw.replace(/^\.[\\/]/, "").replace(/^[\\/]+/, ""); // "." ⇒ project root
  const base = !baseRaw
    ? "<base dir>"
    : isAbsolute || !root
      ? baseRaw
      : localPart && localPart !== "."
        ? `${root}\\${localPart}`
        : root;
  return `${base.replace(/[\\/]$/, "")}\\${folder}`;
}

// A stored path is absolute / machine-specific (Windows drive `C:\`, UNC `\\`, or
// POSIX root `/`) rather than project-relative. Templates keep only relative paths;
// absolute ones are stripped when genericizing - they don't carry across projects.
function isAbsolutePath(p: string): boolean {
  return /^([a-zA-Z]:[\\/]|[\\/]{2}|\/)/.test(p.trim());
}

// Store a picked path **relative** to the project when it's inside it (profiles are
// committed + shared across machines), else keep the absolute/direct path. Uses
// forward slashes so the stored value is cross-platform.
function toProjectRelative(picked: string, projectRoot: string | undefined): string {
  if (!projectRoot) return picked;
  const norm = (s: string) => s.replace(/\\/g, "/").replace(/\/+$/, "");
  const p = norm(picked);
  const root = norm(projectRoot);
  if (p.toLowerCase() === root.toLowerCase()) return ".";
  if (p.toLowerCase().startsWith(root.toLowerCase() + "/")) return "./" + p.slice(root.length + 1);
  return picked; // outside the project → keep the direct/absolute path
}

const PHASE_BLURB: Record<PhaseId, string> = {
  build: "Compiles the target (on by default). Toggle off to reuse existing binaries (-skipbuild).",
  cook: "Cooks content for the platform. Toggle off to reuse an existing cook (-skipcook).",
  stage: "Assembles the ship-shaped tree. Pak and Archive run inside Stage, so turning Stage off disables both.",
  pak: "Bundles staged files into .pak / IoStore containers. Requires Stage.",
  archive: "Copies the finished build to the output destination. Requires Stage.",
  copyExtras: "Copies project files into the finished build. From is project-relative; To is build-output-relative (“.” = build root).",
  steamUpload: "Uploads the archived build to Steam via steamcmd. Runs after Copy Extras; requires Archive. Set the App ID + depots and log in below.",
  // Implicit preflight (not shown as a phase island); the blurb only satisfies the exhaustive map.
  steamLogin: "Signs in to Steam before the build so an interactive login never interrupts a finished build.",
  cleanup: "Purges chosen footprint categories after a successful build to reclaim disk.",
};

export function BuildSettingsWindow() {
  const [project, setProject] = useState<DetectedProject | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [profiles, setProfiles] = useState<BuildConfig[]>([]);
  const [templates, setTemplates] = useState<BuildConfig[]>([]);
  const [registry, setRegistry] = useState<PhaseInfo[]>([]);
  // PhaseId → registry entry, for the data-driven enabled/locked derivation.
  const phaseById = useMemo(() => new Map(registry.map((p) => [p.id, p])), [registry]);

  const [draft, setDraft] = useState<Draft | null>(null);
  const [saved, setSaved] = useState<string>(""); // JSON of the last-saved draft
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [errors, setErrors] = useState<string[]>([]);
  const [flash, setFlash] = useState<string | null>(null);

  const [createOpen, setCreateOpen] = useState(false);
  const [createName, setCreateName] = useState("");
  const [createSource, setCreateSource] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<{ title: string; body: string; action: () => void } | null>(null);

  const dirty = draft != null && JSON.stringify(draft) !== saved;

  // ── initial load ────────────────────────────────────────────────────────────
  useEffect(() => {
    (async () => {
      const proj = await currentProject().catch(() => null);
      setProject(proj);
      const reg = await phaseRegistry().catch(() => []);
      setRegistry(reg);
      setCollapsed(new Set(reg.map((p) => p.id))); // every phase island starts collapsed
      if (proj) {
        const [ps, ts] = await Promise.all([listProfiles().catch(() => []), listTemplates().catch(() => [])]);
        setProfiles(ps);
        setTemplates(ts);
        if (ps[0]) selectInto(ps[0]);
      }
      setLoaded(true);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function selectInto(p: BuildConfig) {
    const d = toDraft(p);
    setDraft(d);
    setSaved(JSON.stringify(d));
    setErrors([]);
  }

  function requestSelect(p: BuildConfig) {
    if (draft && p.id === draft.id) return;
    if (dirty) {
      setConfirm({
        title: "Discard unsaved changes?",
        body: `“${draft!.name}” has unsaved changes. Switching profiles will lose them.`,
        action: () => selectInto(p),
      });
    } else {
      selectInto(p);
    }
  }

  // Apply a mutation to a deep clone of the current draft. Runs `fn` SYNCHRONOUSLY
  // in the event handler (not inside a setState updater), so handlers can safely
  // read `e.currentTarget` - React nulls `currentTarget` after dispatch and
  // StrictMode double-invokes functional updaters, which together caused a
  // "Cannot read properties of null (reading 'value')" crash. JSON round-trip is a
  // safe deep clone of the pure-data draft.
  const update = (fn: (d: Draft) => void) => {
    if (!draft) return;
    const next = JSON.parse(JSON.stringify(draft)) as Draft;
    fn(next);
    setDraft(next);
  };

  async function refreshProfiles(selectId?: string) {
    const ps = await listProfiles().catch(() => []);
    setProfiles(ps);
    void emitProfilesChanged(); // covers create + clone (both route through here)
    if (selectId) {
      const found = ps.find((p) => p.id === selectId);
      if (found) selectInto(found);
    }
  }

  async function save() {
    if (!draft) return;
    try {
      await saveProfile(draft as BuildConfig);
      setSaved(JSON.stringify(draft));
      setErrors([]);
      flashMsg("Saved.");
      const ps = await listProfiles().catch(() => []);
      setProfiles(ps);
      void emitProfilesChanged(); // notify the main window (Build tab) to re-fetch
    } catch (e) {
      setErrors(e instanceof IpcError ? e.message.split("\n") : [String(e)]);
    }
  }

  function discard() {
    if (saved) selectInto(JSON.parse(saved) as BuildConfig);
  }

  function flashMsg(m: string) {
    setFlash(m);
    setTimeout(() => setFlash(null), 2200);
  }

  async function doCreate() {
    if (!createSource || !createName.trim()) return;
    const sep = createSource.indexOf(":");
    const kind = createSource.slice(0, sep) as "template" | "clone";
    const sourceId = createSource.slice(sep + 1);
    try {
      const p = await createProfile({ name: createName.trim(), from: kind, sourceId });
      setCreateOpen(false);
      setCreateName("");
      setCreateSource(null);
      await refreshProfiles(p.id);
    } catch (e) {
      setErrors(e instanceof IpcError ? e.message.split("\n") : [String(e)]);
      setCreateOpen(false);
    }
  }

  async function makeTemplate(p: BuildConfig) {
    const d = toDraft(p);
    // Genericize for reuse: drop only the bits that don't carry across projects.
    // Absolute paths (output base dir, Copy Extras sources) are machine-specific, so
    // strip them; relative paths resolve per-project and are kept.
    const keptExtras = d.phases.copyExtras.items.filter((i) => !isAbsolutePath(i.from));
    const tmpl: BuildConfig = {
      ...(d as BuildConfig),
      id: `tmpl-${Date.now().toString(36)}`,
      name: d.name,
      target: null,
      basedOnTemplate: null,
      builtin: false,
      output: { ...d.output, baseDir: isAbsolutePath(d.output.baseDir) ? "" : d.output.baseDir },
      phases: {
        ...d.phases,
        cook: { ...d.phases.cook, maps: "all" },
        // Keep relative Copy Extras (portable across projects); absolute sources dropped above.
        copyExtras: { enabled: d.phases.copyExtras.enabled && keptExtras.length > 0, items: keptExtras },
        // Steam upload is project-specific: App ID, depots, description, and set-live branch
        // all target one Steam app. Reset to a generic, disabled state so a template never
        // carries one project's Steam config into another (which would push to the WRONG
        // app/depot). Preview is a non-project-specific structural flag, so it's kept.
        steamUpload: {
          ...d.phases.steamUpload,
          enabled: false,
          appId: "",
          description: "",
          branch: "",
          depots: [],
        },
      },
    };
    try {
      await saveTemplate(tmpl);
      setTemplates(await listTemplates().catch(() => templates));
      flashMsg(`Saved “${d.name}” as a template.`);
    } catch (e) {
      setErrors(e instanceof IpcError ? e.message.split("\n") : [String(e)]);
    }
  }

  async function clone(p: BuildConfig) {
    const dup = await duplicateProfile(p.id).catch((e) => {
      setErrors(e instanceof IpcError ? e.message.split("\n") : [String(e)]);
      return null;
    });
    if (dup) await refreshProfiles(dup.id);
  }

  function confirmDelete(p: BuildConfig) {
    setConfirm({
      title: "Delete profile?",
      body: `“${p.name}” will be permanently removed from .uep/profiles/.`,
      action: async () => {
        await deleteProfile(p.id).catch(() => {});
        const ps = await listProfiles().catch(() => []);
        setProfiles(ps);
        void emitProfilesChanged();
        if (draft?.id === p.id) {
          if (ps[0]) selectInto(ps[0]);
          else setDraft(null);
        }
      },
    });
  }

  // Delete a user template (built-ins are backend-protected). Profiles already made
  // from it are self-contained copies, so they're unaffected.
  function confirmDeleteTemplate(t: BuildConfig) {
    setConfirm({
      title: "Delete template?",
      body: `“${t.name}” will be permanently removed. Profiles already created from it are unaffected.`,
      action: async () => {
        try {
          await deleteTemplate(t.id);
        } catch (e) {
          setErrors(e instanceof IpcError ? e.message.split("\n") : [String(e)]);
        }
        const ts = await listTemplates().catch(() => templates);
        setTemplates(ts);
        // If the deleted template was the selected source, fall back to another.
        if (createSource === `template:${t.id}`) {
          setCreateSource(ts[0] ? `template:${ts[0].id}` : null);
        }
      },
    });
  }

  const toggleCollapse = (id: string) =>
    setCollapsed((s) => {
      const n = new Set(s);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });

  // ── render ──────────────────────────────────────────────────────────────────
  return (
    // Fixed viewport height: the top bar, profile rail, and bottom Save/Discard bar
    // stay put - only the settings panel scrolls.
    <Box style={{ height: "100vh", overflow: "hidden", background: tokens.page, display: "flex", flexDirection: "column" }}>
      {/* top bar */}
      <Group
        h={64}
        px={20}
        wrap="nowrap"
        className="uep-chrome"
        style={{ background: tokens.surface, borderBottom: `1px solid ${tokens.border}`, flexShrink: 0 }}
      >
        <Box style={{ color: tokens.ink, display: "grid", placeItems: "center" }}>
          <IconAdjustments size={22} stroke={1.8} />
        </Box>
        <Text fw={700} fz={18} c={tokens.ink}>
          Build Settings
        </Text>
      </Group>

      {/* body: profile rail + editor */}
      <Box style={{ flex: 1, minHeight: 0, display: "flex" }}>
        <ProfileRail
          profiles={profiles}
          selectedId={draft?.id ?? null}
          disabled={!project}
          onSelect={requestSelect}
          onAdd={() => {
            setCreateSource(templates[0] ? `template:${templates[0].id}` : null);
            setCreateOpen(true);
          }}
          onMakeTemplate={makeTemplate}
          onClone={clone}
          onDelete={confirmDelete}
        />

        <Box style={{ flex: 1, minWidth: 0, overflow: "auto" }}>
          {!loaded ? null : !project ? (
            <Notice
              title="No project open"
              body="Open a project from the main window first. Build Settings edits that project's profiles."
            />
          ) : !draft ? (
            <Notice
              title="No profiles yet"
              body="Use + in the profile list to create one from a template (Development / Shipping) or by cloning an existing profile."
            />
          ) : (
            <Stack gap={14} p={20} maw={940}>
              <ProfileIdentity draft={draft} project={project} update={update} />
              {registry.map((info) => (
                <Island
                  key={info.id}
                  info={info}
                  locked={isLocked(info.id, draft, phaseById)}
                  enabled={isEnabled(info.id, draft, phaseById)}
                  open={!collapsed.has(info.id)}
                  onToggleOpen={() => toggleCollapse(info.id)}
                  onToggleEnabled={(on) => {
                    update((d) => setPhaseEnabled(d, info.id, on));
                    // Toggling a phase on reveals its settings; off folds them away.
                    setCollapsed((s) => {
                      const n = new Set(s);
                      on ? n.delete(info.id) : n.add(info.id);
                      return n;
                    });
                  }}
                >
                  <PhaseBody id={info.id} draft={draft} project={project} update={update} />
                </Island>
              ))}
            </Stack>
          )}
        </Box>
      </Box>

      {/* bottom action bar */}
      <Group
        h={56}
        px={20}
        justify="space-between"
        wrap="nowrap"
        style={{ background: tokens.surface, borderTop: `1px solid ${tokens.border}`, flexShrink: 0 }}
      >
        <Group gap={14} wrap="nowrap" style={{ minWidth: 0 }}>
          {dirty && (
            <Group gap={7} wrap="nowrap">
              <Box style={{ width: 8, height: 8, borderRadius: "50%", background: tokens.warn }} />
              <Text fz={13} c={tokens.text}>
                Unsaved changes
              </Text>
            </Group>
          )}
          {flash && (
            <Text fz={13} c={tokens.successText}>
              {flash}
            </Text>
          )}
          {errors.length > 0 && (
            <Text fz={12.5} c={tokens.danger} truncate>
              {errors.join(" · ")}
            </Text>
          )}
        </Group>
        <Group gap={10} wrap="nowrap">
          <Button variant="default" disabled={!dirty} onClick={discard}>
            Discard
          </Button>
          <Button disabled={!draft || !dirty} onClick={save}>
            Save
          </Button>
        </Group>
      </Group>

      {/* create-profile modal */}
      <Modal opened={createOpen} onClose={() => setCreateOpen(false)} title="New profile" centered>
        <Stack gap={12}>
          <Select
            label="Reference from"
            placeholder="Pick a source"
            value={createSource}
            onChange={setCreateSource}
            data={templates.map((t) => ({ value: `template:${t.id}`, label: t.builtin ? `${t.name} (built-in)` : t.name }))}
            renderOption={({ option }) => {
              // Inline delete for user templates; built-ins are read-only (no trash).
              const tmpl = templates.find((t) => t.id === option.value.slice(option.value.indexOf(":") + 1));
              return (
                <Group justify="space-between" wrap="nowrap" gap={6} style={{ width: "100%" }}>
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{option.label}</span>
                  {tmpl && !tmpl.builtin && (
                    <Tooltip label="Delete template" withArrow position="right">
                      <ActionIcon
                        variant="subtle"
                        color="red"
                        size="sm"
                        aria-label={`Delete ${option.label}`}
                        onMouseDown={(e) => {
                          // Don't let the trash select the option or close the dropdown.
                          e.stopPropagation();
                          e.preventDefault();
                        }}
                        onClick={(e) => {
                          e.stopPropagation();
                          confirmDeleteTemplate(tmpl);
                        }}
                      >
                        <IconTrash size={14} />
                      </ActionIcon>
                    </Tooltip>
                  )}
                </Group>
              );
            }}
          />
          <TextInput
            label="Name"
            placeholder="e.g. Nightly"
            value={createName}
            onChange={(e) => setCreateName(e.currentTarget.value)}
            data-autofocus
          />
          <Group justify="flex-end" gap={8} mt={4}>
            <Button variant="default" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button disabled={!createSource || !createName.trim()} onClick={doCreate}>
              Create
            </Button>
          </Group>
        </Stack>
      </Modal>

      {/* confirm modal (discard / delete). zIndex sits above the New-profile Select's
          dropdown (Mantine popover = 300) so the confirm isn't hidden behind an open
          dropdown when deleting a template inline. */}
      <Modal opened={confirm != null} onClose={() => setConfirm(null)} title={confirm?.title} centered zIndex={400}>
        <Text fz={13} c={tokens.textMuted}>
          {confirm?.body}
        </Text>
        <Group justify="flex-end" gap={8} mt={18}>
          <Button variant="default" onClick={() => setConfirm(null)}>
            Cancel
          </Button>
          <Button
            style={dangerSolidButton()}
            onClick={() => {
              confirm?.action();
              setConfirm(null);
            }}
          >
            Confirm
          </Button>
        </Group>
      </Modal>
    </Box>
  );
}

// Flip a phase's `enabled`. Indexed by id - `PhaseId` is camelCase and matches the
// `phases` field names, so no per-phase branching.
function setPhaseEnabled(d: Draft, id: PhaseId, on: boolean) {
  d.phases[id as ConfigPhaseId].enabled = on;
}

// ── profile rail ──────────────────────────────────────────────────────────────
function ProfileRail({
  profiles,
  selectedId,
  disabled,
  onSelect,
  onAdd,
  onMakeTemplate,
  onClone,
  onDelete,
}: {
  profiles: BuildConfig[];
  selectedId: string | null;
  disabled: boolean;
  onSelect: (p: BuildConfig) => void;
  onAdd: () => void;
  onMakeTemplate: (p: BuildConfig) => void;
  onClone: (p: BuildConfig) => void;
  onDelete: (p: BuildConfig) => void;
}) {
  return (
    <Box
      style={{
        width: 300,
        flexShrink: 0,
        borderRight: `1px solid ${tokens.border}`,
        background: tokens.surface,
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Group justify="space-between" px={18} py={14} wrap="nowrap">
        <Text fz={10.5} fw={700} c={tokens.textDim} style={{ letterSpacing: 0.6 }}>
          PROFILES
        </Text>
        <Tooltip label="New profile" withArrow>
          <ActionIcon radius="md" disabled={disabled} onClick={onAdd} aria-label="New profile">
            <IconPlus size={16} />
          </ActionIcon>
        </Tooltip>
      </Group>
      <Box style={{ flex: 1, overflow: "auto" }}>
        {profiles.length === 0 ? (
          <Text fz={12} c={tokens.textDim} px={18} py={10}>
            {disabled ? "Open a project to manage profiles." : "No profiles yet. Add one with +."}
          </Text>
        ) : (
          <Stack gap={0} px={10} py={4}>
            {profiles.map((p) => {
              const active = p.id === selectedId;
              return (
                <Box
                  key={p.id}
                  className="uep-hoverable"
                  onClick={() => onSelect(p)}
                  style={{
                    position: "relative",
                    padding: "10px 12px",
                    borderRadius: 9,
                    cursor: "pointer",
                    background: active ? tokens.hover : undefined,
                    boxShadow: active ? `inset 3px 0 0 0 ${tokens.accent}` : undefined,
                  }}
                >
                  <Group justify="space-between" wrap="nowrap" gap={6}>
                    <Box style={{ minWidth: 0 }}>
                      <Text fz={14} fw={600} c={tokens.text} truncate>
                        {p.name}
                      </Text>
                      <Text fz={11} c={tokens.textDim} truncate>
                        {subtitleOf(p)}
                      </Text>
                    </Box>
                    {/* stopPropagation wrapper: opening the ⋯ menu must not also select the row */}
                    <Box onClick={(e) => e.stopPropagation()} style={{ flexShrink: 0, display: "flex" }}>
                      <Menu position="bottom-end" withArrow shadow="md">
                        <Menu.Target>
                          <ActionIcon variant="subtle" color="gray" aria-label="Profile actions">
                            <IconDots size={16} />
                          </ActionIcon>
                        </Menu.Target>
                        <Menu.Dropdown>
                          <Menu.Item leftSection={<IconTemplate size={14} />} onClick={() => onMakeTemplate(p)}>
                            Make this a template
                          </Menu.Item>
                          <Menu.Item leftSection={<IconCopy size={14} />} onClick={() => onClone(p)}>
                            Clone
                          </Menu.Item>
                          <Menu.Divider />
                          <Menu.Item color="red" leftSection={<IconTrash size={14} />} onClick={() => onDelete(p)}>
                            Delete
                          </Menu.Item>
                        </Menu.Dropdown>
                      </Menu>
                    </Box>
                  </Group>
                </Box>
              );
            })}
          </Stack>
        )}
      </Box>
    </Box>
  );
}

// ── profile identity card ─────────────────────────────────────────────────────
function ProfileIdentity({
  draft,
  project,
  update,
}: {
  draft: Draft;
  project: DetectedProject;
  update: (fn: (d: Draft) => void) => void;
}) {
  const targetData = useMemo(() => {
    const pkg = project.targets.filter((t) => ["Game", "Client", "Server"].includes(t.targetType));
    return [
      { value: "", label: "Auto-detect (first packageable)" },
      ...pkg.map((t) => ({ value: t.name, label: `${t.name} · ${t.targetType}` })),
    ];
  }, [project]);

  return (
    <Paper withBorder radius="md" p="md">
      <Text fw={600} fz={14} c={tokens.ink} mb={12}>
        Profile
      </Text>
      <Stack gap={12}>
        <TextInput
          label="Name"
          value={draft.name}
          onChange={(e) => update((d) => void (d.name = e.currentTarget.value))}
        />
        <Group grow align="flex-start">
          <Select
            label="Platform"
            value={draft.platform}
            onChange={(v) => v && update((d) => void (d.platform = v as Platform))}
            data={["Win64", "Linux", "Mac"]}
            allowDeselect={false}
          />
          <Select
            label="Target"
            value={draft.target ?? ""}
            onChange={(v) => update((d) => void (d.target = v ? v : null))}
            data={targetData}
            allowDeselect={false}
          />
        </Group>
      </Stack>
    </Paper>
  );
}

// ── collapsible phase island ──────────────────────────────────────────────────
function Island({
  info,
  locked,
  enabled,
  open,
  onToggleOpen,
  onToggleEnabled,
  children,
}: {
  info: PhaseInfo;
  locked: boolean;
  enabled: boolean;
  open: boolean;
  onToggleOpen: () => void;
  onToggleEnabled: (on: boolean) => void;
  children: ReactNode;
}) {
  return (
    <Paper withBorder radius="md">
      <Group
        justify="space-between"
        px="md"
        py={12}
        wrap="nowrap"
        className="uep-hoverable"
        // enabled phases tint the header light green so active steps read at a glance
        style={{ cursor: "pointer", borderRadius: 8, background: enabled ? tokens.successBg : undefined }}
        onClick={onToggleOpen}
      >
        <Group gap={12} wrap="nowrap">
          <Switch
            checked={enabled}
            disabled={locked}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => onToggleEnabled(e.currentTarget.checked)}
          />
          <Group gap={6} wrap="nowrap">
            <Text fw={600} fz={14} c={enabled ? tokens.ink : tokens.textMuted}>
              {info.label}
            </Text>
            <Tooltip label={PHASE_BLURB[info.id]} withArrow multiline w={290} position="bottom-start">
              <ActionIcon
                variant="subtle"
                color="gray"
                size="sm"
                radius="xl"
                aria-label={`About the ${info.label} phase`}
                onClick={(e) => e.stopPropagation()}
              >
                <IconInfoCircle size={15} />
              </ActionIcon>
            </Tooltip>
          </Group>
          {locked && (
            <Tooltip label="Pak & Archive run inside Stage. Enable Stage to use them" withArrow>
              <Group gap={3} wrap="nowrap" style={{ color: tokens.textDim }}>
                <IconLock size={12} />
                <Text fz={11} c={tokens.textDim}>
                  needs Stage
                </Text>
              </Group>
            </Tooltip>
          )}
          {info.kind === "app" && (
            <Box
              style={{
                fontSize: 10,
                color: tokens.textMuted,
                background: tokens.neutralBadgeBg,
                border: `1px solid ${tokens.neutralBadgeBorder}`,
                borderRadius: 8,
                padding: "1px 8px",
              }}
            >
              app task
            </Box>
          )}
        </Group>
        <IconChevronDown
          size={18}
          color={tokens.textMuted}
          style={{ transform: open ? "rotate(180deg)" : "none", transition: "transform 120ms" }}
        />
      </Group>
      <Collapse expanded={open}>
        <Box px="md" pb="md" pt={4} style={{ borderTop: `1px solid ${tokens.divider}`, opacity: enabled ? 1 : 0.55 }}>
          {children}
        </Box>
      </Collapse>
    </Paper>
  );
}

// ── per-phase bodies ──────────────────────────────────────────────────────────
function PhaseBody({
  id,
  draft,
  project,
  update,
}: {
  id: PhaseId;
  draft: Draft;
  project: DetectedProject;
  update: (fn: (d: Draft) => void) => void;
}) {
  return (
    <Stack gap={12} pt={12}>
      {id === "build" && (
        <Stack gap={12}>
          <Box>
            <Group gap={6} wrap="nowrap" mb={6}>
              <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
                BUILD CONFIGURATIONS
              </Text>
              <Tooltip
                multiline
                w={330}
                withArrow
                position="bottom-start"
                label="Every ticked configuration is built and staged into the one package: one cook, one .exe per config (e.g. Development alongside Shipping). Each becomes a build-history tag and the folder name joins them. At least one is required."
              >
                <ActionIcon variant="subtle" color="gray" size="xs" radius="xl" aria-label="About build configurations">
                  <IconInfoCircle size={14} />
                </ActionIcon>
              </Tooltip>
            </Group>
            <Paper withBorder radius="sm" p="sm" style={{ background: tokens.surfaceAlt }}>
              <SimpleGrid cols={2} spacing={6} verticalSpacing={6}>
                {ALL_CONFIGS.map((c) => (
                  <Checkbox
                    key={c}
                    size="xs"
                    label={c}
                    checked={draft.configs.includes(c)}
                    onChange={() =>
                      update((d) => {
                        const s = new Set(d.configs);
                        s.has(c) ? s.delete(c) : s.add(c);
                        d.configs = [...s];
                      })
                    }
                  />
                ))}
              </SimpleGrid>
            </Paper>
            {draft.configs.length === 0 && (
              <Text fz={11} c={tokens.danger} mt={6}>
                Select at least one configuration - the profile can't be saved without one.
              </Text>
            )}
          </Box>
          <Checkbox
            label="Clean build (-clean)"
            description="Wipe intermediates first. Mutually exclusive with incremental cook."
            checked={draft.phases.build.clean}
            onChange={(e) => update((d) => void (d.phases.build.clean = e.currentTarget.checked))}
          />
          <Checkbox
            label="No Perforce (-noP4)"
            description="On by default; turn off only if you build with Perforce."
            checked={draft.phases.build.noP4}
            onChange={(e) => update((d) => void (d.phases.build.noP4 = e.currentTarget.checked))}
          />
          <ArgsInput
            label="Additional args"
            placeholder="e.g. -NoCodeSign"
            value={draft.phases.build.additionalArgs}
            onChange={(v) => update((d) => void (d.phases.build.additionalArgs = v))}
          />
        </Stack>
      )}

      {id === "cook" && <CookBody draft={draft} project={project} update={update} />}

      {id === "stage" && (
        <Stack gap={10}>
          <Checkbox
            label="Stage prerequisites installer (-prereqs)"
            checked={draft.phases.stage.prereqs}
            onChange={(e) => update((d) => void (d.phases.stage.prereqs = e.currentTarget.checked))}
          />
          <Checkbox
            label="For distribution (-distribution)"
            description="Store-ready flag, mainly for console/mobile stores."
            checked={draft.phases.stage.forDistribution}
            onChange={(e) => update((d) => void (d.phases.stage.forDistribution = e.currentTarget.checked))}
          />
          <Checkbox
            label="Include debug symbols (.pdb)"
            description="Off emits -nodebuginfo, a footprint win for Shipping."
            checked={draft.phases.stage.debugSymbols}
            onChange={(e) => update((d) => void (d.phases.stage.debugSymbols = e.currentTarget.checked))}
          />
          <Checkbox
            label="Separate debug info (-separatedebuginfo)"
            description="Stage symbols into a separate dir."
            checked={draft.phases.stage.separateDebugInfo}
            onChange={(e) => update((d) => void (d.phases.stage.separateDebugInfo = e.currentTarget.checked))}
          />
          <ArgsInput
            label="Additional args"
            placeholder="e.g. -applocaldirectory=…"
            value={draft.phases.stage.additionalArgs}
            onChange={(v) => update((d) => void (d.phases.stage.additionalArgs = v))}
          />
        </Stack>
      )}

      {id === "pak" && (
        <Stack gap={10}>
          <Checkbox
            label="I/O Store containers (.utoc/.ucas)"
            description="On pulls Pak on (locks this phase)."
            checked={draft.phases.pak.ioStore}
            onChange={(e) => update((d) => void (d.phases.pak.ioStore = e.currentTarget.checked))}
          />
          <Checkbox
            label="Compress content (Oodle)"
            checked={draft.phases.pak.compressed}
            onChange={(e) => update((d) => void (d.phases.pak.compressed = e.currentTarget.checked))}
          />
          <Checkbox
            label="Create native package (-package)"
            description="Produce a platform-native distributable."
            checked={draft.phases.pak.package}
            onChange={(e) => update((d) => void (d.phases.pak.package = e.currentTarget.checked))}
          />
          <ArgsInput
            label="Additional args"
            placeholder="e.g. -compressionformats=…"
            value={draft.phases.pak.additionalArgs}
            onChange={(v) => update((d) => void (d.phases.pak.additionalArgs = v))}
          />
        </Stack>
      )}

      {id === "archive" && <ArchiveBody draft={draft} project={project} update={update} />}

      {id === "copyExtras" && <CopyExtrasBody draft={draft} project={project} update={update} />}

      {id === "steamUpload" && <SteamBody draft={draft} project={project} update={update} />}

      {id === "cleanup" && <CleanupBody draft={draft} update={update} />}
    </Stack>
  );
}

// Left-truncate long map paths (ellipsis on the left) so each checkbox stays inside
// its grid column instead of spilling into the next - the distinctive tail (the map
// name) stays visible, full path on hover via `title`. `minWidth: 0` down the flex
// chain (root → body → labelWrapper → label) is what lets the text shrink and
// ellipsize. The rtl/ellipsis lives on an inner span (MAP_LABEL_SPAN), NOT on
// Mantine's label, so the label's box→text gap (a logical padding-inline-start that
// rtl would otherwise flip to the wrong side) is preserved - matching Cultures.
const MAP_LABEL_STYLES: CheckboxProps["styles"] = {
  root: { minWidth: 0 },
  body: { minWidth: 0 },
  labelWrapper: { minWidth: 0, flex: 1 },
  label: { display: "block", minWidth: 0, overflow: "hidden" },
};

const MAP_LABEL_SPAN: CSSProperties = {
  display: "block",
  overflow: "hidden",
  whiteSpace: "nowrap",
  textOverflow: "ellipsis",
  direction: "rtl",
  textAlign: "left",
};

function CookBody({
  draft,
  project,
  update,
}: {
  draft: Draft;
  project: DetectedProject;
  update: (fn: (d: Draft) => void) => void;
}) {
  const cookAll = draft.phases.cook.maps === "all";
  const allMaps = project.maps.maps;
  // "Cook all maps" (-allmaps) ⇔ every detected map checked. The toggle, the
  // checkboxes, and Select-all stay in lockstep: a full selection normalizes back
  // to "all", a partial one to an explicit list.
  const selected = cookAll ? [...allMaps] : (draft.phases.cook.maps as { list: string[] }).list;
  const selectedSet = new Set(selected);

  const setMaps = (m: CookMaps) => update((d) => void (d.phases.cook.maps = m));
  const applySelection = (list: string[]) =>
    setMaps(allMaps.length > 0 && allMaps.every((m) => list.includes(m)) ? "all" : { list });
  const toggleMap = (m: string) =>
    applySelection(selectedSet.has(m) ? selected.filter((x) => x !== m) : [...selected, m]);

  const cultures = draft.phases.cook.cultures;
  const allCultures = [...new Set([...COMMON_CULTURES, ...cultures])];
  const toggleCulture = (c: string) =>
    update((d) => {
      const set = new Set(d.phases.cook.cultures);
      set.has(c) ? set.delete(c) : set.add(c);
      d.phases.cook.cultures = [...set];
    });

  return (
    <Stack gap={14}>
      <Group grow align="flex-start">
        <Select
          label="Cook mode"
          description="Always cook by the book (on-the-fly is dev-only)."
          value="btb"
          data={[{ value: "btb", label: "By the book" }]}
          disabled
          allowDeselect={false}
        />
        <Select
          label="Incremental cook"
          description={
            draft.phases.cook.incremental !== "none"
              ? "Incremental can miss schema/shader changes, so use a full cook for releases."
              : "Full cook (recommended)."
          }
          value={draft.phases.cook.incremental}
          onChange={(v) => v && update((d) => void (d.phases.cook.incremental = v as IncrementalCookMode))}
          data={[
            { value: "none", label: "Full cook" },
            { value: "modifiedOnly", label: "Iterative (-iterativecooking)" },
            { value: "modifiedAndDependencies", label: "Incremental (-cookincremental, UE 5.6+)" },
          ]}
          allowDeselect={false}
        />
      </Group>

      <Checkbox
        label="Skip editor content (-SkipCookingEditorContent)"
        checked={draft.phases.cook.skipEditorContent}
        onChange={(e) => update((d) => void (d.phases.cook.skipEditorContent = e.currentTarget.checked))}
      />

      <Box>
        <Group justify="space-between" mb={6}>
          <Group gap={6} wrap="nowrap">
            <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
              MAPS TO COOK
            </Text>
            <Tooltip
              multiline
              w={320}
              withArrow
              position="bottom-start"
              label="Ticking a map adds it as an extra map to cook (-map=). Unticking one doesn't guarantee it's left out: a map can still be cooked when it's referenced (soft or hard ref) by cooked content, or forced by the project's Asset Manager rules (AlwaysCook / DirectoriesToAlwaysCook) or default maps."
            >
              <ActionIcon variant="subtle" color="gray" size="xs" radius="xl" aria-label="About maps to cook">
                <IconInfoCircle size={14} />
              </ActionIcon>
            </Tooltip>
          </Group>
          <Group gap={10}>
            <Anchor fz={11} c={tokens.text} onClick={() => setMaps("all")}>
              Select all
            </Anchor>
            <Anchor fz={11} c={tokens.text} onClick={() => setMaps({ list: [] })}>
              None
            </Anchor>
          </Group>
        </Group>
        <Paper withBorder radius="sm" p="sm" style={{ background: tokens.surfaceAlt }}>
          {allMaps.length === 0 ? (
            <Text fz={12} c={tokens.textDim}>
              No maps detected under Content/.
            </Text>
          ) : (
            <SimpleGrid cols={2} spacing={6} verticalSpacing={6}>
              {allMaps.map((m) => {
                const label = m.replace(/^\/Game\//, "");
                return (
                  <Checkbox
                    key={m}
                    size="xs"
                    label={<span style={MAP_LABEL_SPAN}>{label}</span>}
                    title={label}
                    checked={selectedSet.has(m)}
                    onChange={() => toggleMap(m)}
                    styles={MAP_LABEL_STYLES}
                  />
                );
              })}
            </SimpleGrid>
          )}
        </Paper>
      </Box>

      <Box>
        <Group justify="space-between" mb={6}>
          <Group gap={6} wrap="nowrap">
            <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
              COOKED CULTURES
            </Text>
            <Tooltip
              multiline
              w={320}
              withArrow
              position="bottom-start"
              label="Cultures are the languages/locales packaged into the build. Tick the ones to include, or leave them all unticked to cook every culture the project supports."
            >
              <ActionIcon variant="subtle" color="gray" size="xs" radius="xl" aria-label="About cooked cultures">
                <IconInfoCircle size={14} />
              </ActionIcon>
            </Tooltip>
          </Group>
          <Group gap={10}>
            <Anchor fz={11} c={tokens.text} onClick={() => update((d) => void (d.phases.cook.cultures = [...allCultures]))}>
              Select all
            </Anchor>
            <Anchor fz={11} c={tokens.text} onClick={() => update((d) => void (d.phases.cook.cultures = []))}>
              None
            </Anchor>
          </Group>
        </Group>
        <Paper withBorder radius="sm" p="sm" style={{ background: tokens.surfaceAlt }}>
          <SimpleGrid cols={3} spacing={6} verticalSpacing={6}>
            {allCultures.map((c) => (
              <Checkbox key={c} size="xs" label={c} checked={cultures.includes(c)} onChange={() => toggleCulture(c)} />
            ))}
          </SimpleGrid>
        </Paper>
      </Box>

      <ArgsInput
        label="Additional cooker options"
        placeholder="e.g. -CookPartialGC -ddc=…"
        value={draft.phases.cook.additionalOptions}
        onChange={(v) => update((d) => void (d.phases.cook.additionalOptions = v))}
      />
    </Stack>
  );
}

function ArchiveBody({
  draft,
  project,
  update,
}: {
  draft: Draft;
  project: DetectedProject | null;
  update: (fn: (d: Draft) => void) => void;
}) {
  return (
    <Stack gap={12}>
      <Group align="flex-end" gap={8} wrap="nowrap">
        <TextInput
          label="Base directory"
          placeholder="./Builds"
          style={{ flex: 1 }}
          value={draft.output.baseDir}
          onChange={(e) => update((d) => void (d.output.baseDir = e.currentTarget.value))}
        />
        <Button
          variant="default"
          leftSection={<IconFolder size={15} />}
          onClick={async () => {
            const dir = await pickDirectory("Choose the build output base directory", project?.projectRoot);
            if (dir) update((d) => void (d.output.baseDir = toProjectRelative(dir, project?.projectRoot)));
          }}
        >
          Browse
        </Button>
      </Group>
      <TextInput
        label={
          <Group gap={6} wrap="nowrap">
            <span>Folder name</span>
            <Tooltip
              multiline
              w={300}
              withArrow
              label="Tokens: {project} {platform} {config} {profile} {target} {date} {time}. Rendered lowercase; {date}=YYYYMMDD."
            >
              <IconInfoCircle size={14} color={tokens.textDim} />
            </Tooltip>
          </Group>
        }
        value={draft.output.folderTemplate}
        onChange={(e) => update((d) => void (d.output.folderTemplate = e.currentTarget.value))}
      />
      <Box>
        <Text fz={11} fw={600} c={tokens.textMuted} mb={3} style={{ letterSpacing: 0.3 }}>
          PREVIEW
        </Text>
        <CodeBox fz={11.5}>{renderPreview(draft, project)}</CodeBox>
      </Box>
      <ArgsInput
        label="Additional args"
        placeholder="e.g. -CrashReporter"
        value={draft.phases.archive.additionalArgs}
        onChange={(v) => update((d) => void (d.phases.archive.additionalArgs = v))}
      />
    </Stack>
  );
}

/** Mono single-line text input for a phase's verbatim additional-args string. */
function ArgsInput({
  label,
  placeholder,
  value,
  onChange,
}: {
  label: string;
  placeholder: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <TextInput
      label={label}
      placeholder={placeholder}
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
    />
  );
}

function CopyExtrasBody({
  draft,
  project,
  update,
}: {
  draft: Draft;
  project: DetectedProject;
  update: (fn: (d: Draft) => void) => void;
}) {
  const items = draft.phases.copyExtras.items;
  return (
    <Stack gap={10}>
      {items.length === 0 && (
        <Text fz={12} c={tokens.textDim}>
          No mappings yet.
        </Text>
      )}
      {items.map((it, i) => (
        <Group key={i} gap={8} wrap="nowrap" align="center">
          <TextInput
            style={{ flex: 1 }}
            placeholder="./steam_appid.txt"
            value={it.from}
            onChange={(e) => update((d) => void (d.phases.copyExtras.items[i].from = e.currentTarget.value))}
            styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
          />
          <Button
            variant="default"
            px={8}
            onClick={async () => {
              const f = await pickFile("Choose a file to copy into the build", project.projectRoot);
              if (f) update((d) => void (d.phases.copyExtras.items[i].from = toProjectRelative(f, project.projectRoot)));
            }}
          >
            <IconFolder size={15} />
          </Button>
          <Text c={tokens.textDim}>→</Text>
          <TextInput
            style={{ flex: 1 }}
            placeholder="."
            value={it.to}
            onChange={(e) => update((d) => void (d.phases.copyExtras.items[i].to = e.currentTarget.value))}
            styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
          />
          <Button
            variant="subtle"
            color="gray"
            px={8}
            onClick={() => update((d) => void d.phases.copyExtras.items.splice(i, 1))}
          >
            <IconX size={15} />
          </Button>
        </Group>
      ))}
      <Button
        variant="default"
        size="compact-sm"
        leftSection={<IconPlus size={14} />}
        style={{ alignSelf: "flex-start" }}
        onClick={() => update((d) => void d.phases.copyExtras.items.push({ from: "", to: "." }))}
      >
        Add mapping
      </Button>
    </Stack>
  );
}

// ── Steam upload ──────────────────────────────────────────────────────────────
// Managed fields (App ID / description / branch / preview / depots) live in the draft and
// are written into the committed VDFs on save (preserving any custom keys). The steamcmd
// path + build account are machine-local. There is no in-app login: when a build reaches
// the upload step, steamcmd opens in its own window to sign in only when there's no active
// session (see runner::exec), so no password is ever entered in the app.
type SteamState = { steamcmdPath: string; account: string };

function SteamBody({ draft, project, update }: { draft: Draft; project: DetectedProject; update: (fn: (d: Draft) => void) => void }) {
  const s = draft.phases.steamUpload;
  const [steam, setSteam] = useState<SteamState>({ steamcmdPath: "", account: "" });

  useEffect(() => {
    let alive = true;
    loadSteamSettings()
      .then((v) => alive && setSteam({ steamcmdPath: v.steamcmdPath ?? "", account: v.account ?? "" }))
      .catch(() => {});
    return () => void (alive = false);
  }, []);

  const [setupOpen, setSetupOpen] = useState(false);

  const depots = s.depots;
  return (
    <Stack gap={12}>
      <Button
        fullWidth
        leftSection={<IconAdjustments size={16} />}
        onClick={() => setSetupOpen(true)}
      >
        Setup SteamCMD
      </Button>
      <Text fz={11} c={tokens.textDim}>
        Build scripts are written to <code>.uep/steam-config/{draft.id}/</code> and shared with the project. Custom
        VDF keys you add to those files are preserved on save.
      </Text>
      <Divider my={8} />
      <Group grow gap={8} align="flex-start">
        <TextInput
          label="App ID"
          placeholder="e.g. 480"
          value={s.appId}
          onChange={(e) => update((d) => void (d.phases.steamUpload.appId = e.currentTarget.value))}
          styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
        />
        <TextInput
          label="Build description"
          placeholder="shown only in your Steamworks builds page"
          value={s.description}
          onChange={(e) => update((d) => void (d.phases.steamUpload.description = e.currentTarget.value))}
        />
      </Group>

      <TextInput
        label={
          <Group gap={6} wrap="nowrap">
            <span>Set live on branch</span>
            <Tooltip
              multiline
              w={300}
              withArrow
              label="Beta branch to publish to right after upload. Leave empty to upload only. Steam blocks the public 'default' branch from scripts, so publish it manually in Steamworks."
            >
              <IconInfoCircle size={14} color={tokens.textDim} />
            </Tooltip>
          </Group>
        }
        placeholder="(leave empty to upload only)"
        value={s.branch}
        onChange={(e) => update((d) => void (d.phases.steamUpload.branch = e.currentTarget.value))}
        error={
          s.branch.trim().toLowerCase() === "default"
            ? 'The public "default" branch can\'t be set from a script; publish it manually in Steamworks.'
            : undefined
        }
      />

      <Checkbox
        label="Preview"
        description="Dry run: validates the upload without sending anything."
        checked={s.preview}
        onChange={(e) => update((d) => void (d.phases.steamUpload.preview = e.currentTarget.checked))}
      />

      <Divider my={8} />

      <Box>
        <Group gap={6} wrap="nowrap" mb={6}>
          <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
            DEPOTS
          </Text>
          <Tooltip multiline w={280} withArrow label="Each depot uploads a slice of the archived build. Subpath is content-root-relative (“.” = the whole build). Refine file mappings/exclusions directly in the depot VDF.">
            <IconInfoCircle size={14} color={tokens.textDim} />
          </Tooltip>
        </Group>
        <Stack gap={8}>
          {depots.length === 0 && (
            <Text fz={12} c={tokens.textDim}>
              No depots yet.
            </Text>
          )}
          {depots.map((dep, i) => (
            <Group key={i} gap={8} wrap="nowrap" align="center">
              <TextInput
                style={{ width: 130 }}
                placeholder="depot id"
                value={dep.depotId}
                onChange={(e) => update((d) => void (d.phases.steamUpload.depots[i].depotId = e.currentTarget.value))}
                styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
              />
              <TextInput
                style={{ flex: 1 }}
                placeholder="content subpath (default .)"
                value={dep.path}
                onChange={(e) => update((d) => void (d.phases.steamUpload.depots[i].path = e.currentTarget.value))}
                styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
              />
              <Button
                variant="subtle"
                color="gray"
                px={8}
                onClick={() => update((d) => void d.phases.steamUpload.depots.splice(i, 1))}
              >
                <IconX size={15} />
              </Button>
            </Group>
          ))}
          <Button
            variant="default"
            size="compact-sm"
            leftSection={<IconPlus size={14} />}
            style={{ alignSelf: "flex-start" }}
            onClick={() => update((d) => void d.phases.steamUpload.depots.push({ depotId: "", path: "." }))}
          >
            Add depot
          </Button>
        </Stack>
      </Box>

      <SteamSettingsModal
        opened={setupOpen}
        project={project}
        value={steam}
        onClose={() => setSetupOpen(false)}
        onSaved={(next) => {
          setSteam(next);
          setSetupOpen(false);
        }}
      />
    </Stack>
  );
}

// steamcmd path + build account - machine-local (git-ignored `.uep/steam-config/local.json`),
// opened from the "Setup SteamCMD" button. No password: sign-in happens in steamcmd's own
// window at the build's Steam Login step. Shows a live setup + sign-in status (runs
// `steamcmd +login <account> +quit`). The steamcmd path is stored project-relative when inside
// the project.
function SteamSettingsModal({
  opened,
  project,
  value,
  onClose,
  onSaved,
}: {
  opened: boolean;
  project: DetectedProject;
  value: SteamState;
  onClose: () => void;
  onSaved: (s: SteamState) => void;
}) {
  const [s, setS] = useState<SteamState>(value);
  const [status, setStatus] = useState<SteamStatus | null>(null);
  const [checking, setChecking] = useState(false);

  // Read the live setup + sign-in status. Reads the PERSISTED machine-local settings
  // (steam_status loads them from disk) and never writes, so testing can't clobber the
  // saved values - only the explicit Save button persists, so Cancel truly discards.
  const check = async () => {
    setChecking(true);
    try {
      setStatus(await steamStatus());
    } catch (e) {
      setStatus({ steamcmdFound: false, loggedIn: false, message: e instanceof IpcError ? e.message : String(e) });
    } finally {
      setChecking(false);
    }
  };

  // "Sign in": open steamcmd in its own console for interactive sign-in (Steam Guard code
  // or phone approval). Detached - once signed in there, the status updates via Recheck.
  // Does not persist the modal's settings (only Save does); it signs in against the saved
  // steamcmd path/account.
  const signIn = async () => {
    try {
      await steamOpenLoginTerminal();
    } catch (e) {
      setStatus({ steamcmdFound: false, loggedIn: false, message: e instanceof IpcError ? e.message : String(e) });
    }
  };

  useEffect(() => {
    if (opened) {
      setS(value);
      setStatus(null);
      void check();
    }
  }, [opened]);

  return (
    <Modal opened={opened} onClose={onClose} title="Setup SteamCMD" centered>
      <Stack gap={12}>
        <Group align="flex-end" gap={8} wrap="nowrap">
          <TextInput
            label="steamcmd.exe path"
            placeholder="C:\\steamcmd\\steamcmd.exe"
            style={{ flex: 1 }}
            value={s.steamcmdPath}
            onChange={(e) => setS({ ...s, steamcmdPath: e.currentTarget.value })}
            styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
          />
          <Button
            variant="default"
            leftSection={<IconFolder size={15} />}
            onClick={async () => {
              const f = await pickFile("Locate steamcmd.exe", project.projectRoot);
              // Store relative when it's inside the project (portable); absolute otherwise.
              if (f) setS({ ...s, steamcmdPath: toProjectRelative(f, project.projectRoot) });
            }}
          >
            Browse
          </Button>
        </Group>
        <Group align="flex-end" gap={8} wrap="nowrap">
          <TextInput
            label="Steam build account"
            placeholder="build account name"
            style={{ flex: 1 }}
            value={s.account}
            onChange={(e) => setS({ ...s, account: e.currentTarget.value })}
          />
          <Button variant="default" disabled={!s.account.trim()} onClick={signIn}>
            Sign in
          </Button>
        </Group>

        <Paper withBorder radius="sm" p="sm" style={{ background: tokens.surfaceAlt }}>
          <Group justify="space-between" align="center" mb={checking || status ? 8 : 0}>
            <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
              STATUS
            </Text>
            <Button variant="default" size="compact-sm" loading={checking} onClick={() => void check()}>
              Recheck
            </Button>
          </Group>
          {checking && (
            <Text fz={12} c={tokens.textDim}>
              Checking steamcmd… (first run may update steamcmd)
            </Text>
          )}
          {!checking &&
            status &&
            (status.loggedIn ? (
              <Text fz={12} fw={600} c="teal">
                ✓ Signed in as {s.account.trim()}
              </Text>
            ) : (
              <Stack gap={4}>
                <Text fz={12} fw={600} c={status.steamcmdFound ? "teal" : "red"}>
                  {status.steamcmdFound ? "✓ steamcmd found" : "✗ steamcmd not found"}
                </Text>
                {status.steamcmdFound ? (
                  <Text fz={12} fw={600} c="red">
                    ✗ Not signed in
                  </Text>
                ) : (
                  status.message && (
                    <Text fz={11} c={tokens.textDim}>
                      {status.message}
                    </Text>
                  )
                )}
              </Stack>
            ))}
        </Paper>

        <Text fz={11} c={tokens.textDim}>
          Machine-local, git-ignored. No password here: the build's Steam Login step opens steamcmd to sign in when
          needed (enter the code or approve on your phone). Use a dedicated build account.
        </Text>
        <Group justify="flex-end" gap={8}>
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => {
              void saveSteamSettings(s as SteamLocalSettings).catch(() => {});
              onSaved(s);
            }}
          >
            Save
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

// Mirrors the Clean tab / footprint::rules. Save = Staged/Cooked/Shader; Binaries and
// Intermediate each split Game vs Plugin; Derived data cache is standalone. The editor
// cache (Intermediate "Other") is deliberately NOT offered here - wiping it forces a full
// editor recompile, which an automatic on-success clean should never do; it's tab-only.
const CLEANUP_GROUPS = [
  { label: "Binaries", game: "binariesGame", plugin: "binariesPlugin" },
  { label: "Intermediate", game: "intermediateGame", plugin: "intermediatePlugin" },
] as const;
const ALL_CLEANUP_CATS: CleanupCategory[] = [
  "staged",
  "cooked",
  "shader",
  "binariesGame",
  "binariesPlugin",
  "intermediateGame",
  "intermediatePlugin",
  "derivedData",
];

function CleanupBody({ draft, update }: { draft: Draft; update: (fn: (d: Draft) => void) => void }) {
  const cats = draft.phases.cleanup.categories;
  const toggle = (c: CleanupCategory) =>
    update((d) => {
      const set = new Set(d.phases.cleanup.categories);
      set.has(c) ? set.delete(c) : set.add(c);
      d.phases.cleanup.categories = [...set];
    });
  const box = (c: CleanupCategory, label: string) => (
    <Checkbox size="xs" label={label} checked={cats.includes(c)} onChange={() => toggle(c)} />
  );
  return (
    <Stack gap={12}>
      <Box>
        <Group justify="space-between" mb={6}>
          <Text fz={11} fw={600} c={tokens.textMuted} style={{ letterSpacing: 0.3 }}>
            CATEGORIES TO PURGE
          </Text>
          <Group gap={10}>
            <Anchor fz={11} c={tokens.text} onClick={() => update((d) => void (d.phases.cleanup.categories = [...ALL_CLEANUP_CATS]))}>
              Select all
            </Anchor>
            <Anchor fz={11} c={tokens.text} onClick={() => update((d) => void (d.phases.cleanup.categories = []))}>
              None
            </Anchor>
          </Group>
        </Group>
        <Paper withBorder radius="sm" p="sm" style={{ background: tokens.surfaceAlt }}>
          <Stack gap={9}>
            {box("staged", "Staged build")}
            {box("cooked", "Cooked content")}
            {box("shader", "Shader cache")}
            {CLEANUP_GROUPS.map((g) => (
              <Box key={g.label}>
                <Text fz={11.5} fw={600} c={tokens.text}>
                  {g.label}
                </Text>
                <Group gap={24} mt={4} pl={18}>
                  {box(g.game, "Game")}
                  {box(g.plugin, "Plugin")}
                </Group>
              </Box>
            ))}
            {box("derivedData", "Derived data cache")}
          </Stack>
        </Paper>
      </Box>
      <Checkbox
        label="Run only on a successful build"
        description="Keeps artifacts after a failure so it stays debuggable."
        checked={draft.phases.cleanup.onlyOnSuccess}
        onChange={(e) => update((d) => void (d.phases.cleanup.onlyOnSuccess = e.currentTarget.checked))}
      />
    </Stack>
  );
}

// ── shared notice ─────────────────────────────────────────────────────────────
function Notice({ title, body }: { title: string; body: string }) {
  return (
    <Box style={{ height: "100%", minHeight: 360, display: "grid", placeItems: "center" }}>
      <Stack gap={6} align="center" maw={460} px={24}>
        <Text fz={14} fw={600} c={tokens.text} ta="center">
          {title}
        </Text>
        <Text fz={12.5} c={tokens.textDim} ta="center">
          {body}
        </Text>
      </Stack>
    </Box>
  );
}
