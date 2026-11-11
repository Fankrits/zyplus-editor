// Chromium check of what opening one big note costs, and which editor it opens in.
//   node scripts/perf/bigfile.mjs [--dist dist] [--kb 1000] [--json out.json]
// Builds a note of about --kb kilobytes by pasting it into the source editor (a real paste
// event: Playwright's `keyboard.insertText` goes through a DOM-diffing path that takes minutes
// for a megabyte and says nothing about the app), lets it autosave, closes the tab and opens
// the file again from the tree. Reports the editor it opened in, the time until it was usable,
// and the heap and DOM it left behind. Needs Playwright with a Chromium, like web.mjs.
import { createRequire } from "node:module";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const dist = path.resolve(opt("--dist", "dist"));
const kb = Number(opt("--kb", 1000));

let chromium;
for (const base of [process.cwd() + "/", "/opt/node22/lib/node_modules/", "/usr/lib/node_modules/", "/usr/local/lib/node_modules/"]) {
  try {
    ({ chromium } = createRequire(base)("playwright"));
    break;
  } catch {}
}
if (!chromium) {
  console.error("Playwright not found. Install it somewhere reachable (npm i -g playwright) and retry.");
  process.exit(1);
}

const types = { ".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".svg": "image/svg+xml", ".json": "application/json" };
const server = http.createServer((req, res) => {
  const file = path.join(dist, decodeURIComponent(req.url.split("?")[0]));
  const target = fs.existsSync(file) && fs.statSync(file).isFile() ? file : path.join(dist, "index.html");
  res.writeHead(200, { "content-type": types[path.extname(target)] ?? "application/octet-stream" }).end(fs.readFileSync(target));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));

const browser = await chromium.launch({ args: ["--no-sandbox", "--js-flags=--expose-gc"] });
const page = await (await browser.newContext({ viewport: { width: 1200, height: 800 } })).newPage();
const cdp = await page.context().newCDPSession(page);
await cdp.send("Performance.enable");
const metrics = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
const settledMetrics = async () => {
  await cdp.send("HeapProfiler.collectGarbage");
  return metrics();
};

await page.goto(`http://127.0.0.1:${server.address().port}/`, { waitUntil: "networkidle" });
await page.locator(".milkdown .ProseMirror h1").waitFor({ timeout: 30_000 });

// New File creates "Untitled.md" at once; untitled notes save themselves.
await page.keyboard.press("Control+n");
await page.getByRole("tab", { name: /Untitled\.md/ }).waitFor();
await page.getByLabel("Plain text editor").click();
await page.locator(".cm-content").first().waitFor();
const para = "Paragraph with **bold**, *italic* and `code`, long enough to look like real prose in a note. ".repeat(3) + "\n\n";
const text = para.repeat(Math.max(1, Math.round((kb * 1000) / para.length)));
await page.evaluate((t) => {
  const el = document.querySelector(".cm-content");
  const data = new DataTransfer();
  data.setData("text/plain", t);
  el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
}, text);
await page.waitForTimeout(2500); // autosave
await page.keyboard.press("Control+w");
await page.waitForTimeout(500);

const before = await settledMetrics();
const t0 = Date.now();
await page.getByText("Untitled.md", { exact: true }).first().click();
let editor = "none";
await Promise.race([
  page.locator(".milkdown .ProseMirror p").first().waitFor({ timeout: 120_000 }).then(() => (editor = "rich")),
  page.locator(".cm-content").first().waitFor({ timeout: 120_000 }).then(() => (editor = "plain")),
]).catch(() => {});
const openMs = Date.now() - t0;
await page.waitForTimeout(1500);
const after = await settledMetrics();
await browser.close();
server.close();

const out = {
  fileKB: Math.round(text.length / 1000),
  editor,
  openMs,
  heapGrowthMB: Math.round(((after.JSHeapUsedSize - before.JSHeapUsedSize) / 1048576) * 10) / 10,
  domNodes: after.Nodes,
  layoutS: Math.round((after.LayoutDuration - before.LayoutDuration) * 100) / 100,
};
for (const [k, v] of Object.entries(out)) console.log(k.padEnd(14), v);
if (opt("--json", null)) fs.writeFileSync(opt("--json"), JSON.stringify(out, null, 2));
