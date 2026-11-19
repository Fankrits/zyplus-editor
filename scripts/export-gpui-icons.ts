/**
 * Writes the Hugeicons the app uses as standalone SVG files
 * (assets/icons/<kebab-name>.svg). GPUI paints an SVG as an alpha
 * mask tinted by the element's text colour, so strokes are plain black here.
 *
 *   cd scripts && bun install && bun export-gpui-icons.ts
 */
import { mkdirSync, writeFileSync } from "node:fs";
import * as icons from "@hugeicons/core-free-icons";

const NAMES = [
  "Alert01Icon", "AlertCircleIcon", "ArrowDown01Icon", "ArrowRight01Icon", "ArrowUp01Icon",
  "Cancel01Icon", "CenterFocusIcon", "CheckmarkCircle02Icon", "ClipboardIcon", "Copy01Icon",
  "Delete02Icon", "ExternalLinkIcon", "File01Icon", "FileAddIcon", "FileExportIcon",
  "FileTextIcon", "FloppyDiskIcon", "Folder01Icon", "FolderAddIcon", "FolderOpenIcon",
  "InformationCircleIcon", "KeyboardIcon", "ListTreeIcon", "MoreHorizontalIcon",
  "MultiplicationSignIcon", "PaintBoardIcon", "Pdf01Icon", "PencilEdit01Icon", "PlusSignIcon",
  "PuzzleIcon", "Search01Icon", "SearchAddIcon", "SearchMinusIcon", "Settings01Icon",
  "SidebarLeftIcon", "SourceCodeIcon", "Table01Icon", "TextFontIcon", "TrashIcon",
  // Rich editor: selection toolbar, slash menu, link popover, table and code controls.
  "TextBoldIcon", "TextItalicIcon", "TextStrikethroughIcon", "Link01Icon", "Unlink01Icon",
  "Heading01Icon", "Heading02Icon", "Heading03Icon", "LeftToRightListBulletIcon",
  "LeftToRightListNumberIcon", "CheckListIcon", "QuoteDownIcon", "Image01Icon", "CodeSquareIcon",
  "Chart01Icon", "MinusSignIcon", "RowInsertIcon", "ColumnInsertIcon", "RowDeleteIcon",
  "ColumnDeleteIcon", "Tick02Icon", "InformationCircleIcon", "FullScreenIcon",
];

const kebab = (s: string) => s.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
const attr = (k: string, v: unknown) => `${kebab(k)}="${String(v).replace("currentColor", "#000")}"`;

const out = new URL("../assets/icons/", import.meta.url);
mkdirSync(out, { recursive: true });

for (const name of NAMES) {
  const icon = (icons as Record<string, unknown>)[name] as [string, Record<string, unknown>][] | undefined;
  if (!icon) throw new Error(`missing icon ${name}`);
  const body = icon
    .map(([tag, attrs]) => {
      const { key: _key, ...rest } = attrs;
      const hasFill = "fill" in rest;
      const props = Object.entries(rest).map(([k, v]) => attr(k, v));
      if (!hasFill && !("stroke" in rest)) props.push('stroke="#000"');
      return `<${tag} ${props.join(" ")}/>`;
    })
    .join("");
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" ` +
    `fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${body}</svg>\n`;
  const file = `${kebab(name.replace(/Icon$/, ""))}.svg`;
  writeFileSync(new URL(file, out), svg);
}
console.log(`wrote ${NAMES.length} icons`);
