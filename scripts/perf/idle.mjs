// What a focused-but-untouched editor costs while the app sits idle, in Chromium.
//   node scripts/perf/idle.mjs [--dist dist] [--settle 25] [--window 15] [--json out.json]
// Types a few characters, then waits --settle seconds with no input (past the point the caret
// stops blinking), then measures a --window-second stretch of nothing happening:
//   frames   compositor frames Chromium produced (screencast frames: one per visual change). A
//            blinking caret produces a couple a second; a solid caret produces none. This is the
//            closest thing to GPU/compositor wake-ups a headless run can count.
//   cpu      CPU seconds spent by every Chromium process (browser, renderer, GPU, utility), as a
//            percentage of one core, over the window.
// Needs Playwright with a Chromium, like web.mjs.
import { createRequire } from "node:module";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const dist = path.resolve(opt("--dist", "dist"));
const settle = Number(opt("--settle", 25));
const windowS = Number(opt("--window", 15));

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

const browser = await chromium.launch({ args: ["--no-sandbox"] });
const page = await (await browser.newContext({ viewport: { width: 1200, height: 800 } })).newPage();
const cdp = await page.context().newCDPSession(page);

// Chromium reports every process it owns (browser, renderer, GPU, utility) with its CPU time.
const browserCdp = await browser.newBrowserCDPSession();
const treeCpuSeconds = async () => {
  const { processInfo } = await browserCdp.send("SystemInfo.getProcessInfo");
  return processInfo.reduce((sum, p) => sum + p.cpuTime, 0);
};

await page.goto(`http://127.0.0.1:${server.address().port}/`, { waitUntil: "networkidle" });
await page.locator(".milkdown .ProseMirror h1").waitFor({ timeout: 30_000 });
await page.locator(".milkdown .ProseMirror p").first().click();
await page.keyboard.type("hello", { delay: 30 });
await page.waitForTimeout(settle * 1000);

let frames = 0;
cdp.on("Page.screencastFrame", (f) => {
  frames++;
  cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId }).catch(() => {});
});
await cdp.send("Page.startScreencast", { format: "jpeg", quality: 10, everyNthFrame: 1 });
const running = await page.evaluate(() => document.getAnimations().filter((a) => a.playState === "running").length);
if (process.env.DEBUG_ANIM) console.log(JSON.stringify(await page.evaluate(() => document.getAnimations().map((a) => ({ n: a.animationName, s: a.playState, t: a.currentTime, it: a.effect?.getComputedTiming().iterations, cur: a.effect?.getComputedTiming().currentIteration })))));
const c0 = await treeCpuSeconds();
const t0 = Date.now();
await page.waitForTimeout(windowS * 1000);
const c1 = await treeCpuSeconds();
const secs = (Date.now() - t0) / 1000;
await cdp.send("Page.stopScreencast");
await browser.close();
server.close();

const out = {
  settleS: settle,
  windowS: Math.round(secs),
  runningAnimations: running,
  frames,
  framesPerSecond: Math.round((frames / secs) * 100) / 100,
  treeCpuPercent: Math.round(((c1 - c0) / secs) * 10000) / 100,
};
for (const [k, v] of Object.entries(out)) console.log(k.padEnd(18), v);
if (opt("--json", null)) fs.writeFileSync(opt("--json"), JSON.stringify(out, null, 2));
