import { GlobalWindow } from "happy-dom";

// Ensure DOM environment exists for React testing
if (typeof globalThis.document === "undefined") {
  const win = new GlobalWindow();
  for (const key of Object.getOwnPropertyNames(win)) {
    if (!(key in globalThis)) {
      try {
        (globalThis as any)[key] = (win as any)[key];
      } catch {}
    }
  }
  (globalThis as any).window = globalThis;
  (globalThis as any).document = win.document;
}

import { describe, it, expect, beforeEach } from "bun:test";
import { act, fireEvent, render } from "@testing-library/react";
import { WorkspaceProvider, useWorkspace } from "../src/state/workspaceStore";
import * as fs from "../src/lib/fs";
import { SettingsModal } from "../src/components/SettingsModal";

const PATH = "/demo-workspace/notes.md";

/** Drives the provider from inside, the way the editor and settings modal do. */
function Harness({ onReady }: { onReady: (ctx: ReturnType<typeof useWorkspace>) => void }) {
  const ctx = useWorkspace();
  onReady(ctx);
  return null;
}

async function mountWorkspace() {
  let ctx!: ReturnType<typeof useWorkspace>;
  await act(async () => {
    render(
      <WorkspaceProvider>
        <Harness onReady={(c) => (ctx = c)} />
      </WorkspaceProvider>,
    );
  });
  await act(async () => {
    ctx.dispatch({
      type: "OPEN_TAB",
      tab: {
        id: PATH,
        filePath: PATH,
        title: "notes.md",
        content: await fs.readTextFile(PATH),
        isDirty: false,
        mode: "rich",
      },
    });
  });
  return () => ctx;
}

async function typeInto(ctx: () => ReturnType<typeof useWorkspace>, content: string) {
  await act(async () => {
    ctx().dispatch({ type: "UPDATE_TAB_CONTENT", id: PATH, content });
  });
}

const settle = (ms: number) => act(async () => await new Promise((r) => setTimeout(r, ms)));

describe("autosave", () => {
  beforeEach(async () => {
    localStorage.clear();
    await fs.writeTextFile(PATH, "original\n");
  });

  it("writes dirty tabs to disk once typing stops", async () => {
    const ctx = await mountWorkspace();
    await act(async () => ctx().setIsAutosaveEnabled(true));

    await typeInto(ctx, "autosaved\n");
    expect(ctx().state.tabs[0].isDirty).toBe(true);

    await settle(1200);
    expect(await fs.readTextFile(PATH)).toBe("autosaved\n");
    expect(ctx().state.tabs[0].isDirty).toBe(false);
  });

  it("leaves the file alone while autosave is off", async () => {
    const ctx = await mountWorkspace();
    await typeInto(ctx, "unsaved\n");
    await settle(1200);
    expect(await fs.readTextFile(PATH)).toBe("original\n");
    expect(ctx().state.tabs[0].isDirty).toBe(true);
  });

  it("persists the setting across a remount", async () => {
    const ctx = await mountWorkspace();
    await act(async () => ctx().setIsAutosaveEnabled(true));
    await settle(50);

    const reopened = await mountWorkspace();
    expect(reopened().isAutosaveEnabled).toBe(true);
  });
});

describe("settings toggle", () => {
  it("flips autosave from the Settings modal", async () => {
    localStorage.clear();
    let ctx!: ReturnType<typeof useWorkspace>;
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(
        <WorkspaceProvider>
          <Harness onReady={(c) => (ctx = c)} />
          <SettingsModal isOpen onClose={() => {}} />
        </WorkspaceProvider>,
      );
    });

    const toggle = view.getByRole("switch", { name: /save automatically/i });
    expect(ctx.isAutosaveEnabled).toBe(false);

    await act(async () => fireEvent.click(toggle));
    expect(ctx.isAutosaveEnabled).toBe(true);
    await settle(50);
    expect(JSON.parse(localStorage.getItem("zyplus:workspace-session")!).isAutosaveEnabled).toBe(true);

    await act(async () => fireEvent.click(toggle));
    expect(ctx.isAutosaveEnabled).toBe(false);
  });
});
