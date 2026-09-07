import { describe, it, expect } from "bun:test";
import { act, render } from "@testing-library/react";
import { WorkspaceProvider, useWorkspace } from "../src/state/workspaceStore";
import { SESSION_STORAGE_KEY } from "../src/lib/sessionStorage";

const PATH = "/demo-workspace/notes.md";

function Harness({ onReady }: { onReady: (ctx: ReturnType<typeof useWorkspace>) => void }) {
  const ctx = useWorkspace();
  onReady(ctx);
  return null;
}

describe("session persistence", () => {
  it("does not rewrite localStorage while typing", async () => {
    localStorage.clear();
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
          content: "",
          savedContent: "",
          isDirty: false,
          mode: "rich",
        },
      });
    });

    // The session records paths and modes, never content, so typing must not touch
    // localStorage — those writes are synchronous and used to run on every keystroke.
    const setItem = localStorage.setItem.bind(localStorage);
    let writes = 0;
    localStorage.setItem = (key: string, value: string) => {
      if (key === SESSION_STORAGE_KEY) writes++;
      setItem(key, value);
    };
    try {
      for (const text of ["a", "ab", "abc", "abcd"]) {
        await act(async () => {
          ctx.dispatch({ type: "UPDATE_TAB_CONTENT", id: PATH, content: text });
        });
      }
    } finally {
      localStorage.setItem = setItem;
    }

    expect(writes).toBe(0);
  });
});
