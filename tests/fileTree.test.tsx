import { describe, it, expect, beforeAll, afterAll, beforeEach } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { WorkspaceProvider, useWorkspaceActions } from "../src/state/workspaceStore";
import { FileTree } from "../src/components/Sidebar/FileTree";
import * as fs from "../src/lib/fs";

// react-arborist virtualises rows against the container's size, and happy-dom has no layout,
// so report a size the way a real ResizeObserver would.
const RealResizeObserver = globalThis.ResizeObserver;
beforeAll(() => {
  globalThis.ResizeObserver = class {
    constructor(private cb: ResizeObserverCallback) {}
    observe() {
      this.cb([{ contentRect: { width: 300, height: 600 } } as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});
afterAll(() => {
  globalThis.ResizeObserver = RealResizeObserver;
});

describe("file tree (react-arborist)", () => {
  beforeEach(() => localStorage.clear());

  it("shows a project's first level, opens a folder on click, and reports a file opened", async () => {
    await fs.writeTextFile("/ftree/proj/a/inner.md", "x");
    await fs.writeTextFile("/ftree/proj/top.md", "x");
    const opened: string[] = [];
    let actions!: ReturnType<typeof useWorkspaceActions>;
    function Harness() {
      actions = useWorkspaceActions();
      return <FileTree onOpenFile={(path) => opened.push(path)} onRequestCreate={() => {}} />;
    }
    render(
      <WorkspaceProvider>
        <Harness />
      </WorkspaceProvider>,
    );
    await act(async () => {
      await actions.addFolder("/ftree/proj");
    });

    expect(screen.getByText("a")).toBeTruthy();
    expect(screen.getByText("top.md")).toBeTruthy();
    expect(screen.queryByText("inner.md")).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByText("a"));
    });
    expect(await screen.findByText("inner.md")).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByText("top.md"));
    });
    expect(opened).toEqual(["/ftree/proj/top.md"]);
  });
});
