import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { WorkspaceProvider } from "../src/state/workspaceStore";
import { SettingsModal } from "../src/components/SettingsModal";
import { getTextSize, resetTextSizeCache, setTextSize, stepTextSize } from "../src/lib/textSize";

async function openSettings() {
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(
      <WorkspaceProvider>
        <SettingsModal isOpen onClose={() => {}} />
      </WorkspaceProvider>,
    );
  });
  return view;
}

describe("text size setting", () => {
  // Testing Library only unmounts what a test rendered if its auto-cleanup hook happened to be registered
  // by the first file that imported it, which depends on the order bun loads the files in. Without this
  // each test left its Settings modal in the shared DOM, and the next one found several "Increase text
  // size" buttons (and so did whichever file ran after this one).
  afterEach(() => cleanup());

  beforeEach(() => {
    cleanup();
    localStorage.clear();
    resetTextSizeCache();
    document.documentElement.style.removeProperty("--editor-text-scale");
  });

  it("shows the current size and changes it with the buttons", async () => {
    const view = await openSettings();
    const value = () => view.getByTestId("text-size-value").textContent;
    expect(value()).toBe("100%");

    await act(async () => fireEvent.click(view.getByRole("button", { name: /increase text size/i })));
    expect(value()).toBe("110%");
    expect(getTextSize()).toBe(110);
    expect(document.documentElement.style.getPropertyValue("--editor-text-scale")).toBe("1.1");

    await act(async () => fireEvent.click(view.getByRole("button", { name: /decrease text size/i })));
    await act(async () => fireEvent.click(view.getByRole("button", { name: /decrease text size/i })));
    expect(value()).toBe("90%");
  });

  it("follows the keyboard shortcuts, which share its store", async () => {
    const view = await openSettings();
    await act(async () => {
      stepTextSize(1);
      stepTextSize(1);
    });
    expect(view.getByTestId("text-size-value").textContent).toBe("125%");
  });

  it("resets to 100% and disables Reset there", async () => {
    setTextSize(150);
    const view = await openSettings();
    const reset = view.getByRole("button", { name: /^reset$/i }) as HTMLButtonElement;
    expect(reset.disabled).toBe(false);
    await act(async () => fireEvent.click(reset));
    expect(view.getByTestId("text-size-value").textContent).toBe("100%");
    expect((view.getByRole("button", { name: /^reset$/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("disables the buttons at the ends of the range", async () => {
    setTextSize(200);
    const view = await openSettings();
    expect((view.getByRole("button", { name: /increase text size/i }) as HTMLButtonElement).disabled).toBe(true);
    expect((view.getByRole("button", { name: /decrease text size/i }) as HTMLButtonElement).disabled).toBe(false);
  });
});
