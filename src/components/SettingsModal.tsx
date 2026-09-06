import { useState } from "react";
import { Button, Label, Modal, Switch } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  Settings01Icon,
  PaintBoardIcon,
  KeyboardIcon,
  InformationCircleIcon,
  FolderOpenIcon,
  CheckmarkCircle02Icon,
} from "@hugeicons/core-free-icons";
import { useWorkspace } from "../state/workspaceStore";
import * as fs from "../lib/fs";
import { getTheme, setTheme, THEMES, type Theme } from "../lib/theme";
import { formatCombo, SHORTCUTS, TAB_DIGIT_LABEL, isMac } from "../lib/shortcuts";
import { Logo } from "./Logo";

const SECTIONS = [
  { id: "general", label: "General", icon: Settings01Icon },
  { id: "theme", label: "Theme", icon: PaintBoardIcon },
  { id: "shortcuts", label: "Shortcuts", icon: KeyboardIcon },
  { id: "about", label: "About", icon: InformationCircleIcon },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const SHORTCUT_GROUPS = [...new Set(SHORTCUTS.map((s) => s.group))];

function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="flex shrink-0 gap-1">
      {keys.map((k, i) => (
        <kbd
          key={i}
          className="min-w-6 rounded-md border border-border bg-surface-secondary px-1.5 py-0.5 text-center text-xs text-muted"
        >
          {k}
        </kbd>
      ))}
    </span>
  );
}

const THEME_IDS = Object.keys(THEMES) as Theme[];
const THEME_GROUPS = [...new Set(THEME_IDS.map((t) => THEMES[t].group))];

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div>
        <div className="text-sm font-medium text-foreground">{label}</div>
        {hint && <div className="text-xs text-muted">{hint}</div>}
      </div>
      {children}
    </div>
  );
}

/** Miniature app window, so each theme option shows what it does. */
function ThemePreview({ theme }: { theme: Theme }) {
  const [bg, surface, accent] = THEMES[theme].swatch;
  // "System" has no palette of its own; show the two it switches between.
  const halves = theme === "system" ? ["#ffffff", "#18181b"] : [bg, bg];
  return (
    <span
      className="flex h-10 w-full overflow-hidden rounded-md border border-border"
      aria-hidden="true"
    >
      {halves.map((half, i) => (
        <span key={i} className="flex h-full flex-1 gap-0.5 p-1" style={{ background: half }}>
          <span
            className="w-1/3 rounded-sm"
            style={{ background: theme === "system" ? (i ? "#ffffff33" : "#00000018") : surface }}
          />
          <span
            className="flex-1 rounded-sm"
            style={{ background: theme === "system" ? (i ? "#ffffff1a" : "#0000000d") : accent }}
          />
        </span>
      ))}
    </span>
  );
}

export type { SectionId as SettingsSection };

export function SettingsModal({
  isOpen,
  onClose,
  section: controlledSection,
  onSectionChange,
}: {
  isOpen: boolean;
  onClose: () => void;
  section?: SectionId;
  onSectionChange?: (section: SectionId) => void;
}) {
  const { state, dispatch, defaultFolder, setDefaultFolder, isAutosaveEnabled, setIsAutosaveEnabled } =
    useWorkspace();
  const [localSection, setLocalSection] = useState<SectionId>("general");
  const section = controlledSection ?? localSection;
  const setSection = onSectionChange ?? setLocalSection;
  const [theme, setThemeState] = useState<Theme>(getTheme);
  const [defaultAppState, setDefaultAppState] = useState<string | null>(null);

  const pickTheme = (next: Theme) => {
    setTheme(next);
    setThemeState(next);
  };

  const pickDefaultFolder = async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    setDefaultFolder(picked);
    if (!state.roots.includes(picked)) {
      dispatch({ type: "ADD_ROOT", rootPath: picked, node: await fs.readProjectNode(picked) });
    }
  };

  const makeDefaultApp = async () => {
    try {
      await fs.setDefaultMarkdownApp();
      setDefaultAppState("done");
    } catch (err) {
      setDefaultAppState(String(err));
    }
  };

  return (
    <Modal>
      <Modal.Backdrop isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
        <Modal.Container size="lg">
          <Modal.Dialog className="max-w-2xl! overflow-hidden p-0!">
            <div className="flex h-[24rem] max-h-[70vh] flex-col sm:flex-row">
              <nav className="flex shrink-0 gap-1 overflow-x-auto border-b border-border bg-surface-secondary p-2 sm:w-44 sm:flex-col sm:overflow-visible sm:border-b-0 sm:border-r sm:p-3">
                <h2 className="hidden px-2 pt-1 pb-2 text-xs font-medium tracking-wide text-muted uppercase sm:block">
                  Settings
                </h2>
                {SECTIONS.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => setSection(s.id)}
                    aria-current={section === s.id}
                    className={`flex shrink-0 items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm transition-colors ${
                      section === s.id
                        ? "bg-accent-soft text-accent-soft-foreground"
                        : "text-muted hover:bg-surface-hover hover:text-foreground"
                    }`}
                  >
                    <HugeiconsIcon icon={s.icon} size={16} />
                    {s.label}
                  </button>
                ))}
              </nav>

              <div className="flex min-w-0 flex-1 flex-col">
                <header className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3">
                  <h3 className="text-base font-medium text-foreground">
                    {SECTIONS.find((s) => s.id === section)?.label}
                  </h3>
                  <Modal.CloseTrigger className="static!" />
                </header>

                <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 text-sm">
                  {section === "general" && (
                    <div className="flex flex-col gap-5">
                      <Field label="Default folder" hint="Where new files land when nothing is open.">
                        <div className="flex items-center gap-2 rounded-lg border border-border px-3 py-2">
                          <HugeiconsIcon icon={FolderOpenIcon} size={16} className="shrink-0 text-muted" />
                          <span
                            className="min-w-0 flex-1 truncate text-foreground"
                            title={defaultFolder ?? undefined}
                          >
                            {defaultFolder ?? "Not set"}
                          </span>
                          <Button size="sm" variant="secondary" onPress={pickDefaultFolder}>
                            Change
                          </Button>
                        </div>
                      </Field>
                      <Field
                        label="Default Markdown app"
                        hint={
                          isMac
                            ? "Open .md files with Zyplus when you double-click them."
                            : "Set Zyplus as the handler for .md in your system's default apps settings."
                        }
                      >
                        <div className="flex items-center gap-2">
                          <Button
                            size="sm"
                            variant="secondary"
                            isDisabled={!isMac || defaultAppState === "done"}
                            onPress={makeDefaultApp}
                          >
                            {defaultAppState === "done" ? "Zyplus is the default" : "Make default"}
                          </Button>
                          {defaultAppState === "done" && (
                            <HugeiconsIcon icon={CheckmarkCircle02Icon} size={16} className="text-accent" />
                          )}
                          {defaultAppState && defaultAppState !== "done" && (
                            <span className="text-xs text-danger">{defaultAppState}</span>
                          )}
                        </div>
                      </Field>
                      <Field label="Autosave" hint="Saves open files a moment after you stop typing.">
                        <Switch isSelected={isAutosaveEnabled} onChange={setIsAutosaveEnabled}>
                          <Switch.Content>
                            <Switch.Control>
                              <Switch.Thumb />
                            </Switch.Control>
                            <Label>Save automatically</Label>
                          </Switch.Content>
                        </Switch>
                      </Field>
                    </div>
                  )}

                  {section === "theme" && (
                    <Field label="Appearance" hint="System follows your OS setting.">
                      <div className="flex flex-col gap-4">
                        {THEME_GROUPS.map((group) => (
                          <div key={group} className="flex flex-col gap-2">
                            <div className="text-xs font-medium tracking-wide text-muted uppercase">
                              {group}
                            </div>
                            <div className="grid grid-cols-3 gap-3">
                              {THEME_IDS.filter((t) => THEMES[t].group === group).map((t) => (
                                <button
                                  key={t}
                                  type="button"
                                  onClick={() => pickTheme(t)}
                                  aria-pressed={theme === t}
                                  className={`flex flex-col gap-2 rounded-xl border p-2 text-left transition-colors ${
                                    theme === t
                                      ? "border-accent ring-1 ring-accent"
                                      : "border-border hover:bg-surface-hover"
                                  }`}
                                >
                                  <ThemePreview theme={t} />
                                  <span className="truncate px-0.5 text-xs font-medium text-foreground">
                                    {THEMES[t].label}
                                  </span>
                                </button>
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    </Field>
                  )}

                  {section === "shortcuts" && (
                    <div className="flex flex-col gap-5">
                      {SHORTCUT_GROUPS.map((group) => (
                        <div key={group} className="flex flex-col gap-1">
                          <div className="text-xs font-medium tracking-wide text-muted uppercase">
                            {group}
                          </div>
                          <ul className="flex flex-col divide-y divide-border">
                            {SHORTCUTS.filter((s) => s.group === group).map((s) => (
                              <li key={s.id} className="flex items-center justify-between gap-4 py-2.5">
                                <span className="text-foreground">{s.label}</span>
                                <Keys keys={formatCombo(s.combos[0])} />
                              </li>
                            ))}
                            {group === "Tabs" && (
                              <li className="flex items-center justify-between gap-4 py-2.5">
                                <span className="text-foreground">{TAB_DIGIT_LABEL}</span>
                                <Keys keys={[isMac ? "⌘" : "Ctrl", "1–9"]} />
                              </li>
                            )}
                          </ul>
                        </div>
                      ))}
                    </div>
                  )}

                  {section === "about" && (
                    <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
                      <Logo size={40} className="text-foreground" />
                      <div className="text-base font-semibold tracking-tight text-foreground">
                        Zyplus
                      </div>
                      <div className="text-xs text-muted">Version {__APP_VERSION__}</div>
                      <p className="max-w-xs text-xs text-muted">
                        A local-first markdown editor. Your files stay on your machine.
                      </p>
                    </div>
                  )}
                </div>
              </div>
            </div>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}
