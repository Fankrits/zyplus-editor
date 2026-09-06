import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { Input, TextField } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { ArrowUp01Icon, ArrowDown01Icon, Cancel01Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons";
import { SearchQuery, setSearchQuery } from "@codemirror/search";
import type { TabMode } from "../../state/workspaceStore";
import { findPlainEditor } from "./PlainTextEditor";
import { findRichEditor } from "./RichTextEditor";

const HIGHLIGHT_ALL = "zy-search";
const HIGHLIGHT_ACTIVE = "zy-search-active";

// CSS Custom Highlight API — recent TS DOM libs still miss the registry.
const highlights = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights;
const HighlightCtor = (globalThis as unknown as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;

function clearRichHighlights() {
  highlights?.delete(HIGHLIGHT_ALL);
  highlights?.delete(HIGHLIGHT_ACTIVE);
}

/**
 * Matches inside the rendered rich-text DOM. ponytail: per text node, so a match
 * split across inline marks ("**wor**d") is missed — good enough until it isn't.
 */
function findRichMatches(root: HTMLElement, query: string): Range[] {
  const needle = query.toLowerCase();
  const ranges: Range[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = (node.nodeValue ?? "").toLowerCase();
    let i = text.indexOf(needle);
    while (i !== -1) {
      const range = document.createRange();
      range.setStart(node, i);
      range.setEnd(node, i + needle.length);
      ranges.push(range);
      i = text.indexOf(needle, i + needle.length);
    }
  }
  return ranges;
}

interface SearchBarProps {
  mode: TabMode;
  /** The element holding the rendered document, searched directly in rich mode. */
  containerRef: RefObject<HTMLDivElement | null>;
  initialShowReplace?: boolean;
  onClose: () => void;
}

export function SearchBar({ mode, containerRef, initialShowReplace = false, onClose }: SearchBarProps) {
  const [query, setQuery] = useState("");
  const [replacement, setReplacement] = useState("");
  const [showReplace, setShowReplace] = useState(initialShowReplace);
  const [revision, setRevision] = useState(0);
  const [index, setIndex] = useState(0);
  const [total, setTotal] = useState(0);
  const plainMatches = useRef<{ from: number; to: number }[]>([]);
  const richMatches = useRef<Range[]>([]);

  // Keeps the current match after a replace, but never past the end of the list.
  const commit = useCallback((n: number) => {
    setTotal(n);
    setIndex((i) => (i >= n ? 0 : i));
  }, []);

  useEffect(() => setIndex(0), [query, mode]);

  // Collect matches whenever the query (or the document under it) changes.
  useEffect(() => {
    plainMatches.current = [];
    richMatches.current = [];

    if (mode === "plain") {
      clearRichHighlights();
      const view = findPlainEditor();
      if (!view) return setTotal(0);
      const search = new SearchQuery({ search: query, caseSensitive: false });
      view.dispatch({ effects: setSearchQuery.of(search) });
      if (!search.valid) return setTotal(0);
      // ponytail: whole-document scan per keystroke; fine for files that fit in a tab.
      const cursor = search.getCursor(view.state);
      for (let m = cursor.next(); !m.done; m = cursor.next()) {
        plainMatches.current.push({ from: m.value.from, to: m.value.to });
      }
      return commit(plainMatches.current.length);
    }

    const root = containerRef.current;
    if (!root || !query) {
      clearRichHighlights();
      return setTotal(0);
    }
    richMatches.current = findRichMatches(root, query);
    commit(richMatches.current.length);
  }, [query, mode, containerRef, revision, commit]);

  // Reveal (and, in rich mode, paint) the current match.
  useEffect(() => {
    if (mode === "plain") {
      const match = plainMatches.current[index];
      const view = findPlainEditor();
      if (!match || !view) return;
      view.dispatch({ selection: { anchor: match.from, head: match.to }, scrollIntoView: true });
      return;
    }
    const ranges = richMatches.current;
    if (!highlights || !HighlightCtor) return;
    if (ranges.length === 0) return clearRichHighlights();
    highlights.set(HIGHLIGHT_ALL, new HighlightCtor(...ranges));
    highlights.set(HIGHLIGHT_ACTIVE, new HighlightCtor(ranges[index]));
    ranges[index]?.startContainer.parentElement?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [index, total, mode, revision]);

  // Leave no highlight or lingering query behind when the bar closes.
  useEffect(
    () => () => {
      clearRichHighlights();
      const view = findPlainEditor();
      view?.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: "" })) });
    },
    [],
  );

  /** Rewrites the current match (or every match), then re-collects. */
  const replaceMatches = useCallback(
    (all: boolean) => {
      if (total === 0 || query === "") return;

      if (mode === "plain") {
        const view = findPlainEditor();
        if (!view) return;
        const targets = all ? plainMatches.current : [plainMatches.current[index]];
        view.dispatch({
          changes: targets.filter(Boolean).map((m) => ({ from: m.from, to: m.to, insert: replacement })),
        });
      } else {
        const view = findRichEditor();
        if (!view) return;
        const targets = all ? richMatches.current : [richMatches.current[index]];
        const tr = view.state.tr;
        // Back to front, so each replacement leaves the earlier positions untouched.
        for (const range of targets.filter(Boolean).reverse()) {
          const from = view.posAtDOM(range.startContainer, range.startOffset);
          const to = view.posAtDOM(range.endContainer, range.endOffset);
          if (from < 0 || to < from) continue;
          tr.insertText(replacement, from, to);
        }
        if (tr.docChanged) view.dispatch(tr);
      }

      setRevision((r) => r + 1);
    },
    [index, mode, query, replacement, total],
  );

  const step = useCallback(
    (delta: number) => {
      if (total === 0) return;
      setIndex((i) => (i + delta + total) % total);
    },
    [total],
  );

  const inputClass = "h-8 rounded-3xl border-none bg-transparent text-sm shadow-none";
  const iconButtonClass =
    "flex size-7 items-center justify-center rounded-full text-muted no-highlight outline-none hover:bg-foreground/10 focus-visible:status-focused disabled:status-disabled";

  return (
    <div className="absolute right-3 top-3 z-20 flex flex-col gap-1 rounded-3xl border border-border bg-overlay px-1.5 py-1 shadow-lg">
      <div className="flex items-center gap-1">
        <button
          type="button"
          aria-label={showReplace ? "Hide replace" : "Show replace"}
          aria-expanded={showReplace}
          onClick={() => setShowReplace((v) => !v)}
          className={iconButtonClass}
        >
          <HugeiconsIcon
            icon={ArrowRight01Icon}
            size={15}
            strokeWidth={2}
            className={showReplace ? "rotate-90 transition-transform" : "transition-transform"}
          />
        </button>
        <TextField aria-label="Find in document" className="w-40 sm:w-56">
          <Input
            autoFocus
            value={query}
            placeholder="Find"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                step(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
            className={inputClass}
          />
        </TextField>
        <span className="w-14 shrink-0 text-center text-xs tabular-nums text-muted">
          {query === "" ? "" : total === 0 ? "0 results" : `${index + 1}/${total}`}
        </span>
        {[
          { key: "prev", label: "Previous match", icon: ArrowUp01Icon, delta: -1 },
          { key: "next", label: "Next match", icon: ArrowDown01Icon, delta: 1 },
        ].map((btn) => (
          <button
            key={btn.key}
            type="button"
            aria-label={btn.label}
            disabled={total === 0}
            onClick={() => step(btn.delta)}
            className={iconButtonClass}
          >
            <HugeiconsIcon icon={btn.icon} size={15} strokeWidth={2} />
          </button>
        ))}
        <button type="button" aria-label="Close search" onClick={onClose} className={iconButtonClass}>
          <HugeiconsIcon icon={Cancel01Icon} size={15} strokeWidth={2} />
        </button>
      </div>

      {showReplace && (
        <div className="flex items-center gap-1 pl-8">
          <TextField aria-label="Replace with" className="w-40 sm:w-56">
            <Input
              value={replacement}
              placeholder="Replace"
              onChange={(e) => setReplacement(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  replaceMatches(e.shiftKey);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  onClose();
                }
              }}
              className={inputClass}
            />
          </TextField>
          {[
            { key: "one", label: "Replace", all: false },
            { key: "all", label: "All", all: true },
          ].map((btn) => (
            <button
              key={btn.key}
              type="button"
              disabled={total === 0}
              onClick={() => replaceMatches(btn.all)}
              className="h-7 shrink-0 rounded-full px-2.5 text-xs text-muted no-highlight outline-none hover:bg-foreground/10 focus-visible:status-focused disabled:status-disabled"
            >
              {btn.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
