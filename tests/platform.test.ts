import { describe, it, expect } from "bun:test";
import { displayJoin } from "../src/lib/fs";
import { isPdfExportSupported } from "../src/lib/exportPdf";

describe("displayJoin", () => {
  it("keeps a POSIX parent on POSIX separators", () => {
    expect(displayJoin("/Users/me/Documents", "Zyplus")).toBe("/Users/me/Documents/Zyplus");
  });

  it("keeps a Windows parent on backslashes", () => {
    expect(displayJoin("C:\\Users\\me\\Documents", "Zyplus")).toBe(
      "C:\\Users\\me\\Documents\\Zyplus",
    );
  });

  it("does not double a separator the parent already ends with", () => {
    expect(displayJoin("/Users/me/", "Zyplus")).toBe("/Users/me/Zyplus");
    expect(displayJoin("C:\\", "Zyplus")).toBe("C:\\Zyplus");
  });

  it("treats a mixed-separator path as POSIX", () => {
    expect(displayJoin("/mnt/c/Users\\me", "Zyplus")).toBe("/mnt/c/Users\\me/Zyplus");
  });
});

describe("isPdfExportSupported", () => {
  // The browser build prints through the window, so it is always available there;
  // only the native path is macOS-only. Tests run outside Tauri.
  it("is available outside the desktop app", () => {
    expect(isPdfExportSupported()).toBe(true);
  });
});
