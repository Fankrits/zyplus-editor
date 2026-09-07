import { GlobalWindow } from "happy-dom";

/**
 * The one DOM the whole suite shares.
 *
 * `bun test` loads every test file into a single process, so globals installed
 * by one file are visible to all the others. Bootstrapping the DOM per file —
 * guarded on `typeof document === "undefined"`, as this project used to — makes
 * the suite order-dependent: whichever file loads first wins, and any file that
 * installs a *partial* stand-in (a bare `{ documentElement }`, say) silently
 * starves every file loaded after it. Installing it once here, before any test
 * module is imported, is what keeps the run deterministic.
 *
 * Registered as `[test] preload` in bunfig.toml. Tests that need to steer a
 * specific global (`matchMedia`, `fetch`) should override it in `beforeEach`
 * and restore it in `afterEach` rather than reassigning it at module scope.
 */
const win = new GlobalWindow();

for (const key of Object.getOwnPropertyNames(win)) {
  // Anything Bun already provides (navigator, fetch, CustomEvent, …) stays put:
  // those are the implementations the runtime's own machinery is built against.
  if (!(key in globalThis)) {
    try {
      (globalThis as Record<string, unknown>)[key] = (win as unknown as Record<string, unknown>)[key];
    } catch {
      // Some window properties are getter-only; skipping them is fine.
    }
  }
}

// React and Testing Library resolve `window` and `document` independently, so
// both have to point at this one environment.
(globalThis as Record<string, unknown>).window = globalThis;
(globalThis as Record<string, unknown>).document = win.document;

export {};
