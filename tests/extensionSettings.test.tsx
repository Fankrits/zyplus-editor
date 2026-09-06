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
import { WorkspaceProvider } from "../src/state/workspaceStore";
import { SettingsModal } from "../src/components/SettingsModal";
import { extensionManager } from "../src/extensions/extensionManager";

describe("SettingsModal Extensions Tab", () => {
  beforeEach(async () => {
    localStorage.clear();
    await extensionManager.uninstallAndRemove("mermaid");
    await extensionManager.uninstallAndRemove("katex");
  });

  it("renders the Extensions section and allows clicking Install button and toggle", async () => {
    // Mock global fetch for extension download
    const mockBundleCode = `
      export default function() {
        return {
          id: "mermaid",
          renderCodeBlockPreview(lang, content, apply) {
            apply("<svg>mock-mermaid</svg>");
          }
        };
      }
    `;

    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => {
      return new Response(mockBundleCode, {
        status: 200,
        headers: { "Content-Type": "application/javascript" },
      });
    }) as unknown as typeof fetch;

    try {
      let view!: ReturnType<typeof render>;
      await act(async () => {
        view = render(
          <WorkspaceProvider>
            <SettingsModal isOpen section="extensions" onClose={() => {}} />
          </WorkspaceProvider>,
        );
      });

      // Check that Mermaid Diagrams is rendered
      expect(view.getByText("Mermaid Diagrams")).toBeDefined();

      // Check that the switch exists and is clickable
      const toggle = view.getByRole("switch", { name: /enable mermaid diagrams/i });
      expect(toggle).toBeDefined();

      // Click the Install button
      const installButtons = view.getAllByRole("button", { name: /^install$/i });
      expect(installButtons.length).toBeGreaterThanOrEqual(1);

      await act(async () => {
        fireEvent.click(installButtons[0]);
      });

      // Verify extensionManager now has mermaid installed
      expect(extensionManager.getState("mermaid")?.status).toBe("installed");

      // Verify toggle now reflects active state and Remove button appears
      const removeButton = view.getByRole("button", { name: /^remove$/i });
      expect(removeButton).toBeDefined();

      // Click remove
      await act(async () => {
        fireEvent.click(removeButton);
      });

      expect(extensionManager.getState("mermaid")?.status).toBe("uninstalled");

      // Test toggling ON via the Switch directly
      await act(async () => {
        fireEvent.click(toggle);
      });
      expect(extensionManager.getState("mermaid")?.status).toBe("installed");

      // Test toggling OFF via the Switch directly
      await act(async () => {
        fireEvent.click(toggle);
      });
      expect(extensionManager.getState("mermaid")?.status).toBe("uninstalled");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
