import { describe, it, expect, afterEach } from "bun:test";
import { render, waitFor } from "@testing-library/react";
import { ExtensionFileEditor } from "../src/components/Editor/ExtensionFileEditor";
import type { FileEditorOptions } from "../src/extensions/types";
import { extensionManager } from "../src/extensions/extensionManager";
import { isMac } from "../src/lib/platform";

const mod = isMac ? { metaKey: true } : { ctrlKey: true };

// The JSON editor's root stops every keydown, as a widget that owns the keyboard
// does. That left ⌘S dead while it had focus; this stands in for it.
function mountSwallowingEditor() {
  let inner!: HTMLElement;
  extensionManager.loadRuntime = async () => ({
    id: "json",
    mountFileEditor(host) {
      inner = document.createElement("div");
      inner.addEventListener("keydown", (e) => e.stopPropagation());
      host.append(inner);
      return { destroy: () => inner.remove() };
    },
  });
  const view = render(<ExtensionFileEditor extensionId="json" initialValue="" onChange={() => {}} onModeChange={() => {}} />);
  return { view, inner: () => inner };
}

function press(el: HTMLElement, init: KeyboardEventInit) {
  const e = new KeyboardEvent("keydown", { code: "KeyS", key: "s", bubbles: true, cancelable: true, ...init });
  el.dispatchEvent(e);
  return e;
}

describe("ExtensionFileEditor shortcuts", () => {
  const original = extensionManager.loadRuntime;
  const seen: KeyboardEvent[] = [];
  const spy = (e: Event) => {
    seen.push(e as KeyboardEvent);
    e.preventDefault(); // what the app does with a shortcut it handles
  };
  afterEach(() => {
    extensionManager.loadRuntime = original;
    document.removeEventListener("keydown", spy);
    seen.length = 0;
  });

  // Spied on `document`: under bun's DOM `window` is a separate object that happy-dom
  // does not bubble into. In a browser the same event goes on to the app's window handler.
  it("gets app shortcuts out of the editor even though it swallows them", async () => {
    document.addEventListener("keydown", spy);
    const { inner } = mountSwallowingEditor();
    await waitFor(() => expect(inner()).toBeDefined());

    const e = press(inner(), { ...mod });
    expect(seen).toHaveLength(1);
    expect(seen[0].code).toBe("KeyS");
    // The app handled it, so the browser must not also save the page.
    expect(e.defaultPrevented).toBe(true);
  });

  it("leaves find, and every unmodified key, to the editor", async () => {
    document.addEventListener("keydown", spy);
    const { inner } = mountSwallowingEditor();
    await waitFor(() => expect(inner()).toBeDefined());

    press(inner(), { ...mod, code: "KeyF", key: "f" });
    press(inner(), { code: "KeyS", key: "s" });
    expect(seen).toHaveLength(0);
  });
});

describe("ExtensionFileEditor views", () => {
  const original = extensionManager.loadRuntime;
  afterEach(() => {
    extensionManager.loadRuntime = original;
  });

  it("opens in the tab's view, reports the one shown, and follows the tab bar after that", async () => {
    let options!: FileEditorOptions;
    const switched: string[] = [];
    extensionManager.loadRuntime = async () => ({
      id: "json",
      mountFileEditor(_host, o) {
        options = o;
        return { destroy: () => {}, setMode: (m) => switched.push(m) };
      },
    });
    const reported: string[] = [];
    const view = render(
      <ExtensionFileEditor
        extensionId="json"
        initialValue="{}"
        onChange={() => {}}
        mode="table"
        onModeChange={(m) => reported.push(m)}
      />,
    );
    await waitFor(() => expect(options).toBeDefined());

    // Opened in the view the tab already had, not the extension's default.
    expect(options.mode).toBe("table");
    options.onModeChange("text");
    expect(reported).toEqual(["text"]);

    // The user picks another in the tab bar: the editor is told, and not remounted.
    view.rerender(
      <ExtensionFileEditor
        extensionId="json"
        initialValue="{}"
        onChange={() => {}}
        mode="tree"
        onModeChange={(m) => reported.push(m)}
      />,
    );
    expect(switched).toEqual(["tree"]);
  });
});

