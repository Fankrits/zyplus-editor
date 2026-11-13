import { afterEach, describe, expect, it } from "bun:test";
import { act, cleanup, render, screen } from "@testing-library/react";
import { ToastHost } from "../src/components/ToastHost";
import { showToast } from "../src/lib/toasts";

describe("toasts load on demand", () => {
  // See textSizeSettings.test.tsx: do not leave the toast region in the shared DOM for later files.
  afterEach(() => cleanup());

  it("renders nothing until a toast is asked for, then shows it", async () => {
    render(<ToastHost />);
    // No toast region has been mounted yet.
    expect(document.querySelector("[data-react-aria-top-layer], [role=region]")).toBeNull();

    await act(async () => {
      await showToast("info", "Mermaid available", { description: "A dedicated editor exists." });
    });
    // The region lazy-loads, so give it a moment.
    expect(await screen.findByText("Mermaid available", {}, { timeout: 4000 })).toBeTruthy();
  });
});
