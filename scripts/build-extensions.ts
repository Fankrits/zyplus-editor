import fs from "node:fs";
import path from "node:path";

async function buildExtensions() {
  const outdir = path.resolve(import.meta.dirname, "../extensions/dist");
  // Every file here is generated and hashed into checksums.json below, so a file
  // left over from a bundle that no longer exists would be checksummed and shipped.
  fs.rmSync(outdir, { recursive: true, force: true });
  fs.mkdirSync(outdir, { recursive: true });

  const bundle = async (id: string) => {
    console.log(`Building ${id} extension bundle...`);
    const result = await Bun.build({
      entrypoints: [path.resolve(import.meta.dirname, `../src/extensions/bundles/${id}.ts`)],
      outdir,
      target: "browser",
      format: "esm",
      minify: true,
      naming: `${id}.js`,
    });
    if (!result.success) {
      throw new Error(`${id} build failed:\n${result.logs.join("\n")}`);
    }
  };
  const copyCss = (id: string) =>
    fs.copyFileSync(path.resolve(import.meta.dirname, `../src/extensions/bundles/${id}.css`), path.join(outdir, `${id}.css`));

  await bundle("mermaid");
  await bundle("katex");

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

  await bundle("json");
  // The editor's own styles ship inside the bundle; this maps its variables onto the app's theme tokens.
  copyCss("json");

  await bundle("csv");
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

  await bundle("chart");
  await bundle("codefiles");
  copyCss("codefiles");

  // Release builds download these from the repository and run them, so each
  // build carries the hashes of the exact files at its own commit. Hashed as
  // text, the way the app hashes what it downloads.
  const checksums: Record<string, string> = {};
  for (const name of fs.readdirSync(outdir).filter((f) => /\.(js|css)$/.test(f)).sort()) {
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
