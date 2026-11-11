// Chromium check of what opening one big note costs, and which editor it opens in.
//   node scripts/perf/bigfile.mjs [--dist dist] [--kb 1000] [--json out.json]
// Builds a note of about --kb kilobytes by pasting it into the source editor (a real paste
// event: Playwright's `keyboard.insertText` goes through a DOM-diffing path that takes minutes
// for a megabyte and says nothing about the app), lets it autosave, closes the tab and opens
// the file again from the tree. Reports the editor it opened in, the time until it was usable,
// the heap and DOM it left behind, and what typing at the end of it costs (keystroke-to-frame
// latency, long tasks, main-thread time per key). Needs Playwright with a Chromium, like web.mjs.
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
// In-page: long tasks, and keystroke -> next frame latency (as in web.mjs).
await page.addInitScript(() => {
  window.__perf = { long: [], latency: [] };
  new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__perf.long.push(e.duration))).observe({ type: "longtask", buffered: true });
  addEventListener(
    "keydown",
    () => {
      const t0 = performance.now();
      requestAnimationFrame(() => setTimeout(() => window.__perf.latency.push(performance.now() - t0), 0));
    },
    true,
  );
});
const pct = (xs, p) => (xs.length ? [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor((p / 100) * xs.length))] : 0);
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

// Type at the end of the open note.
await page.evaluate(() => { window.__perf.long.length = 0; window.__perf.latency.length = 0; });
const editorSelector = editor === "rich" ? ".milkdown .ProseMirror" : ".cm-content";
await page.locator(editorSelector).first().click();
await page.keyboard.press("Control+End");
await page.waitForTimeout(800);
const typingBefore = await metrics();
await page.keyboard.type("Typing at the end of a big note, one key at a time. ".repeat(2), { delay: 25 });
await page.waitForTimeout(500);
const typingAfter = await metrics();
const perf = await page.evaluate(() => window.__perf);
const keys = perf.latency.length;
await browser.close();
server.close();

const out = {
  fileKB: Math.round(text.length / 1000),
  editor,
  openMs,
  heapGrowthMB: Math.round(((after.JSHeapUsedSize - before.JSHeapUsedSize) / 1048576) * 10) / 10,
  domNodes: after.Nodes,
  layoutS: Math.round((after.LayoutDuration - before.LayoutDuration) * 100) / 100,
  typing: {
    keys,
    p50Ms: Math.round(pct(perf.latency, 50) * 10) / 10,
    p95Ms: Math.round(pct(perf.latency, 95) * 10) / 10,
    maxMs: Math.round(Math.max(0, ...perf.latency) * 10) / 10,
    longTasks: perf.long.length,
    longestTaskMs: Math.round(Math.max(0, ...perf.long)),
    // Main-thread busy time per key, over the typing and the half second after it.
    mainThreadMsPerKey: Math.round(((typingAfter.TaskDuration - typingBefore.TaskDuration) / Math.max(1, keys)) * 10000) / 10,
  },
};
for (const [k, v] of Object.entries(out)) console.log(k.padEnd(14), typeof v === "object" ? JSON.stringify(v) : v);
if (opt("--json", null)) fs.writeFileSync(opt("--json"), JSON.stringify(out, null, 2));
