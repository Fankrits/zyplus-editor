// Launches the desktop binary headless (Xvfb + a private D-Bus session, fresh HOME so every
// run is a first run) and samples the whole process tree: app + WebKit web/network processes.
//   node scripts/perf/desktop.mjs <binary> [--seconds 40] [--json out.json]
//        [--dist <dir> [--project <folder>]]
// Reports peak / final RSS and the CPU spent while booting versus while idle.
//
// With a release binary the app runs its embedded frontend. To compare frontends on one
// binary instead, pass a DEBUG binary (it loads devUrl, http://localhost:1420) and --dist:
// that build output is served there, and --project restores the folder as an open project,
// the way a relaunch would. (A folder given on the command line first fails to open as a
// file, which raises a blocking error dialog, so it can't be used headless.)
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i === -1 ? fallback : args[i + 1];
};
const binary = args.find((a) => !a.startsWith("--") && args.indexOf(a) === 0);
if (!binary) {
  console.error("usage: node scripts/perf/desktop.mjs <binary> [--seconds 40] [--json out.json] [--dist <dir> [--project <folder>]]");
  process.exit(1);
}
const seconds = Number(opt("--seconds", 40));
const idleWindow = 10; // last N seconds count as "idle"
const dist = opt("--dist", null);
const project = opt("--project", null);
const display = ":97";
const home = fs.mkdtempSync(path.join(os.tmpdir(), "zyplus-perf-"));

let server = null;
if (dist) {
  const http = await import("node:http");
  const seed = project
    ? `<script>try{localStorage.setItem("zyplus:workspace-session",${JSON.stringify(
        JSON.stringify({ version: 2, roots: [path.resolve(project)], defaultFolder: null, tabs: [], activeFilePath: null, isSidebarCollapsed: false, isAutosaveEnabled: false }),
      )})}catch(e){}</script>`
    : "";
  const types = { ".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".svg": "image/svg+xml", ".json": "application/json" };
  server = http.createServer((req, res) => {
    const file = path.join(path.resolve(dist), decodeURIComponent(req.url.split("?")[0]));
    const target = fs.existsSync(file) && fs.statSync(file).isFile() ? file : path.join(path.resolve(dist), "index.html");
    let body = fs.readFileSync(target);
    if (target.endsWith("index.html")) body = Buffer.from(body.toString().replace("<head>", `<head>${seed}`));
    res.writeHead(200, { "content-type": types[path.extname(target)] ?? "application/octet-stream" }).end(body);
  }).listen(1420, "127.0.0.1");
}

// A killed Xvfb leaves its lock behind, and the next launch then silently fails.
for (const stale of [`/tmp/.X${display.slice(1)}-lock`, `/tmp/.X11-unix/X${display.slice(1)}`]) fs.rmSync(stale, { force: true });

const xvfb = spawn("Xvfb", [display, "-screen", "0", "1280x800x24"], { stdio: "ignore", detached: true });
await new Promise((r) => setTimeout(r, 800));
const app = spawn("dbus-run-session", ["--", path.resolve(binary)], {
  stdio: "ignore",
  detached: true,
  env: {
    ...process.env,
    DISPLAY: display,
    HOME: home,
    XDG_DATA_HOME: path.join(home, "data"),
    XDG_CONFIG_HOME: path.join(home, "config"),
    XDG_CACHE_HOME: path.join(home, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
  },
});

const TICK = Number(execFileSync("getconf", ["CLK_TCK"]).toString());
const descendants = (root) => {
  const kids = new Map();
  for (const pid of fs.readdirSync("/proc").filter((n) => /^\d+$/.test(n))) {
    try {
      const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
      const ppid = stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1];
      (kids.get(ppid) ?? kids.set(ppid, []).get(ppid)).push(pid);
    } catch {}
  }
  const out = [];
  const walk = (p) => { for (const k of kids.get(String(p)) ?? []) { out.push(k); walk(k); } };
  walk(root);
  return out;
};
const sample = () => {
  let rss = 0, ticks = 0, procs = 0;
  for (const pid of descendants(app.pid)) {
    try {
      const st = fs.readFileSync(`/proc/${pid}/status`, "utf8");
      const name = /Name:\s+(\S+)/.exec(st)?.[1] ?? "";
      if (/^(dbus|Xvfb|bwrap)/.test(name)) continue;
      rss += Number(/VmRSS:\s+(\d+)/.exec(st)?.[1] ?? 0);
      const raw = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
      const stat = raw.slice(raw.lastIndexOf(")") + 2).split(" ");
      ticks += Number(stat[11]) + Number(stat[12]); // utime + stime
      procs++;
    } catch {}
  }
  return { rssMB: rss / 1024, cpuS: ticks / TICK, procs };
};

const samples = [];
const t0 = Date.now();
while ((Date.now() - t0) / 1000 < seconds) {
  await new Promise((r) => setTimeout(r, 500));
  samples.push({ t: (Date.now() - t0) / 1000, ...sample() });
}
for (const p of [app.pid, xvfb.pid]) { try { process.kill(-p, "SIGKILL"); } catch {} }
server?.close();
fs.rmSync(home, { recursive: true, force: true });

const last = samples.at(-1);
if (!last || last.procs === 0) {
  console.error("the app never started (no processes sampled); is port 1420 or display :97 still in use?");
  process.exit(2);
}
const idleStart = samples.find((s) => s.t >= seconds - idleWindow) ?? samples[0];
const result = {
  binary: path.basename(binary),
  project: project ?? null,
  peakRssMB: Math.max(...samples.map((s) => s.rssMB)),
  finalRssMB: last.rssMB,
  processes: last.procs,
  cpuSecondsTotal: last.cpuS,
  idleCpuPercent: ((last.cpuS - idleStart.cpuS) / (last.t - idleStart.t)) * 100,
};
console.log(JSON.stringify(result, null, 2));
const out = opt("--json", null);
if (out) fs.writeFileSync(out, JSON.stringify({ ...result, samples }, null, 2));
process.exit(0);
