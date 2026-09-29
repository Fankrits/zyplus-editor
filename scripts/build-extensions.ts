import fs from "node:fs";
import path from "node:path";

async function buildExtensions() {
  const outdir = path.resolve(import.meta.dirname, "../extensions/dist");
  if (!fs.existsSync(outdir)) {
    fs.mkdirSync(outdir, { recursive: true });
  }

  console.log("Building Mermaid extension bundle...");
  const mermaidBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/mermaid.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "mermaid.js",
  });
  if (!mermaidBuild.success) {
    console.error("Mermaid build failed:", mermaidBuild.logs);
    process.exit(1);
  }

  console.log("Building KaTeX extension bundle...");
  const katexBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/katex.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "katex.js",
  });
  if (!katexBuild.success) {
    console.error("KaTeX build failed:", katexBuild.logs);
    process.exit(1);
  }

  console.log("Preparing KaTeX CSS bundle with inlined fonts...");
  const katexDist = path.resolve(import.meta.dirname, "../node_modules/katex/dist");
  // A missing stylesheet used to be skipped with the build still "successful",
  // shipping math with no styles at all.
  const katexCss = fs.readFileSync(path.join(katexDist, "katex.min.css"), "utf-8");
  // Each @font-face lists woff2, woff and ttf. Every webview the app runs in
  // reads woff2, so that one is inlined and the rest dropped: math then renders
  // offline, and the app's CSP needs no font host. ~400 KB, once, at install.
  let inlined = 0;
  const cssContent = katexCss.replace(/src:([^;}]+)/g, (_, sources: string) => {
    const woff2 = /url\(["']?(?:\.\/)?(fonts\/[^"')]+\.woff2)["']?\)/.exec(sources);
    if (!woff2) return `src:${sources}`;
    inlined++;
    const data = fs.readFileSync(path.join(katexDist, woff2[1])).toString("base64");
    return `src:url(data:font/woff2;base64,${data}) format("woff2")`;
  });
  if (inlined === 0 || /url\((?!data:)/.test(cssContent)) {
    console.error("KaTeX CSS: font URLs were not all inlined; its format may have changed.");
    process.exit(1);
  }
  fs.writeFileSync(path.join(outdir, "katex.css"), cssContent, "utf-8");

  console.log("Building JSON editor extension bundle...");
  const jsonBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/json.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "json.js",
  });
  if (!jsonBuild.success) {
    console.error("JSON editor build failed:", jsonBuild.logs);
    process.exit(1);
  }
  // The editor's own styles ship inside the bundle; this maps its variables onto the app's theme tokens.
  fs.copyFileSync(path.resolve(import.meta.dirname, "../src/extensions/bundles/json.css"), path.join(outdir, "json.css"));

  console.log("Building CSV editor extension bundle...");
  const csvBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/csv.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "csv.js",
  });
  if (!csvBuild.success) {
    console.error("CSV editor build failed:", csvBuild.logs);
    process.exit(1);
  }
  // Tabulator's own CSS (`tabulator.min.css`) is the structural stylesheet its markup
  // requires to lay out as a grid at all (flex, absolute-positioned headers, column
  // widths) — without it the table renders as plain stacked blocks. Our own csv.css is
  // only the HeroUI recoloring on top of it, the same relationship katex.min.css has to
  // its font-inlined build above. Tabulator's own colors are harmless: every one we care
  // about is overridden with `!important`.
  const tabulatorBaseCss = fs.readFileSync(
    path.resolve(import.meta.dirname, "../node_modules/tabulator-tables/dist/css/tabulator.min.css"),
    "utf-8",
  );
  const csvOverrideCss = fs.readFileSync(path.resolve(import.meta.dirname, "../src/extensions/bundles/csv.css"), "utf-8");
  fs.writeFileSync(path.join(outdir, "csv.css"), tabulatorBaseCss + "\n" + csvOverrideCss);

  console.log("Building Charts extension bundle...");
  const chartBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/chart.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "chart.js",
  });
  if (!chartBuild.success) {
    console.error("Chart build failed:", chartBuild.logs);
    process.exit(1);
  }

  console.log("Building Code & Config Files extension bundle...");
  const codefilesBuild = await Bun.build({
    entrypoints: [path.resolve(import.meta.dirname, "../src/extensions/bundles/codefiles.ts")],
    outdir,
    target: "browser",
    format: "esm",
    minify: true,
    naming: "codefiles.js",
  });
  if (!codefilesBuild.success) {
    console.error("Code & Config Files build failed:", codefilesBuild.logs);
    process.exit(1);
  }
  fs.copyFileSync(
    path.resolve(import.meta.dirname, "../src/extensions/bundles/codefiles.css"),
    path.join(outdir, "codefiles.css"),
  );

  // Release builds download these from the repository and run them, so each
  // build carries the hashes of the exact files at its own commit. Hashed as
  // text, the way the app hashes what it downloads.
  const checksums: Record<string, string> = {};
  for (const name of [
    "mermaid.js", "katex.js", "katex.css", "json.js", "json.css", "csv.js", "csv.css", "chart.js",
    "codefiles.js", "codefiles.css",
  ]) {
    const text = fs.readFileSync(path.join(outdir, name), "utf-8");
    checksums[name] = new Bun.CryptoHasher("sha256").update(text).digest("hex");
  }
  fs.writeFileSync(
    path.resolve(import.meta.dirname, "../src/extensions/checksums.json"),
    JSON.stringify(checksums, null, 2) + "\n",
  );

  console.log("Extension bundles built successfully in extensions/dist/");
}

buildExtensions().catch((err) => {
  console.error("Build failed:", err);
  process.exit(1);
});
