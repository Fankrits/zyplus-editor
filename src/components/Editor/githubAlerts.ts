import { $prose } from "@milkdown/kit/utils";
import { Plugin, PluginKey } from "@milkdown/kit/prose/state";
import { Decoration, DecorationSet } from "@milkdown/kit/prose/view";
import type { Node as ProseNode } from "@milkdown/kit/prose/model";

/*
 * GitHub-style alerts: a blockquote starting with "[!NOTE]" renders with an
 * icon + label instead of the literal marker. Decorations only — the marker
 * stays in the document, so the markdown round-trips unchanged.
 */

const ICONS: Record<string, string> = {
  note: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
  tip: '<path d="M9 18h6"/><path d="M10 22h4"/><path d="M15.09 14c.18-.98.65-1.74 1.41-2.5A4.65 4.65 0 0 0 18 8 6 6 0 0 0 6 8c0 1 .23 2.23 1.5 3.5A4.61 4.61 0 0 1 8.91 14"/>',
  important:
    '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M12 7v4"/><path d="M12 15h.01"/>',
  warning:
    '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
  caution:
    '<path d="M2.586 16.726A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2h6.624a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586z"/><path d="M12 8v4"/><path d="M12 16h.01"/>',
};

const MARKER = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*/i;

function badge(kind: string) {
  const el = document.createElement("span");
  el.className = "gh-alert-badge";
  el.contentEditable = "false";
  el.innerHTML =
    `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ` +
    `stroke-linecap="round" stroke-linejoin="round">${ICONS[kind]}</svg>` +
    `<span>${kind.charAt(0).toUpperCase()}${kind.slice(1)}</span>`;
  return el;
}

export function decorate(doc: ProseNode) {
  const decos: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (node.type.name !== "blockquote") return;
    const match = MARKER.exec(node.firstChild?.textContent ?? "");
    if (!match) return false;
    const kind = match[1].toLowerCase();
    // +2 skips into the blockquote and its first paragraph, where the marker text starts.
    const from = pos + 2;
    decos.push(
      Decoration.node(pos, pos + node.nodeSize, { class: `gh-alert gh-alert-${kind}` }),
      Decoration.widget(from, () => badge(kind), { side: -1 }),
      Decoration.inline(from, from + match[0].length, { class: "gh-alert-marker" }),
    );
    return false;
  });
  return DecorationSet.create(doc, decos);
}

export const githubAlerts = $prose(
  () =>
    new Plugin({
      key: new PluginKey("ZYPLUS_GITHUB_ALERTS"),
      // ponytail: rescans the whole doc per state; switch to a mapped state field if
      // huge documents get janky.
      props: { decorations: (state) => decorate(state.doc) },
    }),
);
