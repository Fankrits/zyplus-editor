// Chromium harness for the editor's runtime cost. Serves a build output, then drives it:
//   boot -> type in a small note -> load a big note (many code blocks) and type in it -> idle.
//   node scripts/perf/web.mjs [--dist dist] [--json out.json]
// Needs Playwright with a Chromium; it is deliberately not a dependency of the project.
// Looked up here, then in the usual global spot (PLAYWRIGHT_BROWSERS_PATH picks the browser).
import { createRequire } from "node:module";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const dist = path.resolve(opt("--dist", "dist"));

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
const url = `http://127.0.0.1:${server.address().port}/`;

const browser = await chromium.launch({ args: ["--no-sandbox", "--js-flags=--expose-gc"] });
const page = await (await browser.newContext({ viewport: { width: 1200, height: 800 } })).newPage();
const cdp = await page.context().newCDPSession(page);
await cdp.send("Performance.enable");

// In-page: long tasks, and keystroke -> next frame latency.
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

const metrics = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
const heapMB = async () => {
  await cdp.send("HeapProfiler.collectGarbage");
  return (await metrics()).JSHeapUsedSize / 1048576;
};
const pct = (xs, p) => (xs.length ? [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor((p / 100) * xs.length))] : 0);
const drain = () => page.evaluate(() => { const p = window.__perf; const out = { long: p.long.slice(), latency: p.latency.slice() }; p.long.length = 0; p.latency.length = 0; return out; });

const out = {};

// 1. boot
let t = Date.now();
await page.goto(url, { waitUntil: "networkidle" });
await page.locator(".milkdown .ProseMirror h1").waitFor({ timeout: 30_000 });
out.bootMs = Date.now() - t;
await page.waitForTimeout(1500);
out.heapAfterBootMB = await heapMB();
const boot = await metrics();
out.bootScriptS = boot.ScriptDuration;
out.domNodesBoot = boot.Nodes;
await drain();

// 2. type in a small note (rich editor)
await page.keyboard.press("Control+n");
await page.getByPlaceholder("file-name.md").fill("small");
await page.keyboard.press("Enter");
await page.locator(".milkdown .ProseMirror").waitFor();
await page.locator(".milkdown .ProseMirror").click();
await page.keyboard.type("Typing into a small note, one key at a time. ".repeat(6), { delay: 25 });
let d = await drain();
out.small = { keys: d.latency.length, p50: pct(d.latency, 50), p95: pct(d.latency, 95), max: Math.max(0, ...d.latency), longTasks: d.long.length };

// 3. a big note: 120 sections, each with prose and a code fence
const md = Array.from({ length: 120 }, (_, i) =>
  [`## Section ${i}`, "", `Paragraph ${i} with **bold**, *italic* and \`code\`. `.repeat(3), "", "```js", `const value${i} = ${i}`, `function f${i}() { return value${i} }`, "```", ""].join("\n"),
).join("\n");
await page.keyboard.press("Control+n");
await page.getByPlaceholder("file-name.md").fill("big");
await page.keyboard.press("Enter");
await page.getByLabel("Plain text editor").click();
await page.locator(".cm-content").first().click();
await page.keyboard.insertText(md);
t = Date.now();
await page.getByLabel("Rich text editor").click();
await page.locator(".milkdown-code-block").nth(119).waitFor({ timeout: 60_000 });
out.bigDocMountMs = Date.now() - t;
await page.waitForTimeout(1500);
out.heapBigDocMB = await heapMB();
out.domNodesBigDoc = (await metrics()).Nodes;
await drain();
await page.locator(".milkdown .ProseMirror h2").last().click();
await page.keyboard.press("Control+End");
await page.keyboard.type("Typing at the end of a document with 120 code blocks. ".repeat(4), { delay: 25 });
d = await drain();
out.big = { keys: d.latency.length, p50: pct(d.latency, 50), p95: pct(d.latency, 95), max: Math.max(0, ...d.latency), longTasks: d.long.length, longestTaskMs: Math.max(0, ...d.long) };

// 4. idle: how much of the main thread is busy when nothing is happening. With the editor
// focused the caret's blink animation is all there is; unfocused it should be nothing.
const idle = async () => {
  await page.waitForTimeout(1500);
  const a = await metrics();
  await page.waitForTimeout(6_000);
  const b = await metrics();
  return ((b.TaskDuration - a.TaskDuration) / (b.Timestamp - a.Timestamp)) * 100;
};
out.idleFocusedPercent = await idle();
await page.evaluate(() => document.activeElement?.blur());
out.idleBlurredPercent = await idle();

await browser.close();
server.close();

const f = (n) => (typeof n === "number" ? Math.round(n * 10) / 10 : n);
const rows = [
  ["boot to first note (ms)", out.bootMs],
  ["JS heap after boot (MB)", out.heapAfterBootMB],
  ["script time during boot (s)", out.bootScriptS],
  ["small note: keystroke p50 / p95 / max (ms)", `${f(out.small.p50)} / ${f(out.small.p95)} / ${f(out.small.max)}  (${out.small.longTasks} long tasks)`],
  ["big note (120 code blocks): mount (ms)", out.bigDocMountMs],
  ["big note: JS heap (MB) / DOM nodes", `${f(out.heapBigDocMB)} / ${out.domNodesBigDoc}`],
  ["big note: keystroke p50 / p95 / max (ms)", `${f(out.big.p50)} / ${f(out.big.p95)} / ${f(out.big.max)}  (${out.big.longTasks} long tasks, longest ${f(out.big.longestTaskMs)} ms)`],
  ["idle main thread busy, editor focused / not (%)", `${f(out.idleFocusedPercent)} / ${f(out.idleBlurredPercent)}`],
];
for (const [k, v] of rows) console.log(k.padEnd(46), typeof v === "number" ? f(v) : v);
if (opt("--json", null)) fs.writeFileSync(opt("--json"), JSON.stringify(out, null, 2));
