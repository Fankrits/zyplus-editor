import { GlobalWindow, Event as HappyDomEvent, EventTarget as HappyDomEventTarget } from "happy-dom";

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

/**
 * `globalThis.Event`/`CustomEvent` stay Bun's own native classes (the loop above
 * skips them, since Bun already provides them) — deliberately: `src/lib/commands.ts`'s
 * `emit()` constructs a `CustomEvent` and dispatches it via the bare `window`, which is
 * `globalThis` itself, so both sides of that call are consistently Bun-native today and
 * work correctly.
 *
 * A library that calls `document.createElement(...)` gets a happy-dom element instead,
 * whose `dispatchEvent` is happy-dom's own `EventTarget.prototype` method and checks
 * `instanceof` happy-dom's *own* internal `Event` class — a plain `new Event(...)` at
 * that call site resolves the *same* `globalThis.Event` as everywhere else (Bun-native),
 * so it fails that check even though it's a completely valid Event by any real browser's
 * standard (this is what Tabulator's own `resetScroll()` hits: a bare `new Event('scroll')`
 * dispatched on the table's own div). Forcing `globalThis.Event` to happy-dom's version
 * would flip the break to `emit()` instead (Bun's own native `dispatchEvent` on `window`
 * is just as strict, confirmed directly), so the fix is scoped to happy-dom's element-level
 * dispatch alone: accept anything with a `.type` string even when it isn't `instanceof`
 * happy-dom's Event, by re-wrapping it into a real one first. Real browsers have exactly
 * one Event class and never hit this; it's purely an artifact of Bun and happy-dom
 * co-existing in one process.
 */
type HappyDomTarget = InstanceType<typeof HappyDomEventTarget>;
type HappyDomEventParam = Parameters<HappyDomTarget["dispatchEvent"]>[0];
const originalDispatchEvent = HappyDomEventTarget.prototype.dispatchEvent;
HappyDomEventTarget.prototype.dispatchEvent = function (this: HappyDomTarget, event: HappyDomEventParam) {
  const e = event as unknown as { type?: unknown; bubbles?: boolean; cancelable?: boolean };
  const looksLikeAnEvent = typeof e?.type === "string";
  const needsRewrap = looksLikeAnEvent && !(event instanceof HappyDomEvent);
  const toDispatch = needsRewrap
    ? (new HappyDomEvent(e.type as string, { bubbles: e.bubbles, cancelable: e.cancelable }) as HappyDomEventParam)
    : event;
  return originalDispatchEvent.call(this, toDispatch);
};

export {};
