import { describe, it, expect, beforeEach, afterEach, setSystemTime, jest } from "bun:test";
import { BATCH_MS, LARGE_DOC_CHARS, createChangeReporter } from "../src/components/Editor/changeReporter";
import { flushPendingEdits, hasPendingEdits, registerPendingEdits } from "../src/lib/pendingEdits";

describe("createChangeReporter", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => {
    jest.useRealTimers();
    setSystemTime();
  });

  it("reports every change of a small document at once", () => {
    const seen: string[] = [];
    const r = createChangeReporter((t) => seen.push(t));
    r.docChanged(5, () => "a");
    r.docChanged(6, () => "ab");
    expect(seen).toEqual(["a", "ab"]);
    expect(r.hasPending()).toBe(false);
  });

  it("copies a big document out once per interval, not per keystroke", () => {
    const seen: string[] = [];
    let reads = 0;
    const r = createChangeReporter((t) => seen.push(t));
    for (let i = 0; i < 20; i++) {
      r.docChanged(LARGE_DOC_CHARS + 1, () => {
        reads++;
        return `text${i}`;
      });
    }
    expect(seen).toEqual([]);
    expect(reads).toBe(0);
    expect(r.hasPending()).toBe(true);
    jest.advanceTimersByTime(BATCH_MS);
    // The last read wins: it copies the document as it is when the report is due.
    expect(seen).toEqual(["text19"]);
    expect(reads).toBe(1);
    expect(r.hasPending()).toBe(false);
  });

  it("keeps reporting through continuous typing", () => {
    const seen: string[] = [];
    const r = createChangeReporter((t) => seen.push(t));
    for (let i = 0; i < 3; i++) {
      r.docChanged(LARGE_DOC_CHARS + 1, () => `t${i}`);
      jest.advanceTimersByTime(BATCH_MS);
    }
    expect(seen).toEqual(["t0", "t1", "t2"]);
  });

  it("flush reports a waiting batch immediately, and only once", () => {
    const seen: string[] = [];
    const r = createChangeReporter((t) => seen.push(t));
    r.docChanged(LARGE_DOC_CHARS + 1, () => "big");
    r.flush();
    r.flush();
    jest.advanceTimersByTime(BATCH_MS * 2);
    expect(seen).toEqual(["big"]);
  });

  it("drops a stale batch when the document shrinks back under the limit", () => {
    const seen: string[] = [];
    const r = createChangeReporter((t) => seen.push(t));
    r.docChanged(LARGE_DOC_CHARS + 1, () => "big");
    r.docChanged(10, () => "small");
    jest.advanceTimersByTime(BATCH_MS * 2);
    expect(seen).toEqual(["small"]);
  });
});

describe("pending edits registry", () => {
  it("lets the store settle whatever the mounted editor is holding", () => {
    let pending = true;
    const unregister = registerPendingEdits({
      flush: () => {
        pending = false;
      },
      hasPending: () => pending,
    });
    expect(hasPendingEdits()).toBe(true);
    flushPendingEdits();
    expect(hasPendingEdits()).toBe(false);
    unregister();
    expect(hasPendingEdits()).toBe(false);
  });

  it("does nothing with no editor mounted", () => {
    expect(hasPendingEdits()).toBe(false);
    expect(() => flushPendingEdits()).not.toThrow();
  });
});
