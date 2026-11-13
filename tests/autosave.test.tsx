import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { WorkspaceProvider, useWorkspace } from "../src/state/workspaceStore";
import * as fs from "../src/lib/fs";
import { registerPendingEdits } from "../src/lib/pendingEdits";
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
        savedContent: await fs.readTextFile(PATH),
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
    expect(ctx().state.tabs.find((t) => t.id === PATH)!.isDirty).toBe(true);

    await settle(1200);
    expect(await fs.readTextFile(PATH)).toBe("autosaved\n");
    expect(ctx().state.tabs.find((t) => t.id === PATH)!.isDirty).toBe(false);
  });

  it("leaves the file alone while autosave is off", async () => {
    const ctx = await mountWorkspace();
    await typeInto(ctx, "unsaved\n");
    await settle(1200);
    expect(await fs.readTextFile(PATH)).toBe("original\n");
    expect(ctx().state.tabs.find((t) => t.id === PATH)!.isDirty).toBe(true);
  });

  it("persists the setting across a remount", async () => {
    const ctx = await mountWorkspace();
    await act(async () => ctx().setIsAutosaveEnabled(true));
    await settle(50);

    const reopened = await mountWorkspace();
    expect(reopened().isAutosaveEnabled).toBe(true);
  });
});

describe("untitled files", () => {
  it("save themselves even with autosave off", async () => {
    const ctx = await mountWorkspace();
    const path = await fs.freePath("/demo-workspace", "Untitled.md");
    expect(path).toBe("/demo-workspace/Untitled.md");
    await fs.createFile(path);
    expect(await fs.freePath("/demo-workspace", "Untitled.md")).toBe("/demo-workspace/Untitled 2.md");
    await act(async () => {
      ctx().dispatch({
        type: "OPEN_TAB",
        tab: { id: path, filePath: path, title: "Untitled.md", content: "", savedContent: "", isDirty: false, mode: "rich" },
      });
      ctx().dispatch({ type: "UPDATE_TAB_CONTENT", id: path, content: "jotted\n" });
    });
    await settle(1200);
    expect(await fs.readTextFile(path)).toBe("jotted\n");
    expect(ctx().state.tabs.find((t) => t.id === path)!.isDirty).toBe(false);
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

describe("getState settles edits an editor is still batching", () => {
  afterEach(() => cleanup());

  it("flushes a pending batch into the store before returning state", async () => {
    await fs.writeTextFile(PATH, "original\n");
    const ctx = await mountWorkspace();
    let pending: string | null = "typed in the last 150 ms\n";
    const unregister = registerPendingEdits({
      hasPending: () => pending !== null,
      flush: () => {
        const text = pending;
        pending = null;
        if (text !== null) ctx().dispatch({ type: "UPDATE_TAB_CONTENT", id: PATH, content: text });
      },
    });
    try {
      // The store has not seen the edit yet; asking for state must make it current first.
      expect(ctx().state.tabs.find((t) => t.id === PATH)?.content).toBe("original\n");
      let content = "";
      await act(async () => {
        content = ctx().getState().tabs.find((t) => t.id === PATH)?.content ?? "";
      });
      expect(content).toBe("typed in the last 150 ms\n");
      expect(pending).toBeNull();
    } finally {
      unregister();
    }
  });

  it("leaves state alone when nothing is waiting", async () => {
    await fs.writeTextFile(PATH, "original\n");
    const ctx = await mountWorkspace();
    const before = ctx().getState();
    expect(ctx().getState()).toBe(before);
  });
});
