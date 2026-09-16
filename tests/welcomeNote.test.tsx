import { describe, it, expect, beforeEach } from "bun:test";
import { act, render } from "@testing-library/react";
import { WorkspaceProvider, useWorkspace } from "../src/state/workspaceStore";
import * as fs from "../src/lib/fs";

const NOTE = "/Zyplus/Welcome.md";

async function mount() {
  let ctx!: ReturnType<typeof useWorkspace>;
  function Harness() {
    ctx = useWorkspace();
    return null;
  }
  await act(async () => {
    render(<WorkspaceProvider><Harness /></WorkspaceProvider>);
  });
  return () => ctx;
}

describe("first run", () => {
  beforeEach(() => localStorage.clear());

  it("opens Welcome.md in the default folder", async () => {
    await fs.deletePath(NOTE, false);
    const ctx = await mount();
    expect(ctx().state.roots).toContain("/Zyplus");
    expect(ctx().state.activeTabId).toBe(NOTE);
    expect(await fs.readTextFile(NOTE)).toContain("# Welcome to Zyplus");
  });

  it("keeps a Welcome.md the user already edited", async () => {
    await fs.writeTextFile(NOTE, "mine\n");
    const ctx = await mount();
    expect(ctx().state.tabs.find((t) => t.id === NOTE)?.content).toBe("mine\n");
  });
});
