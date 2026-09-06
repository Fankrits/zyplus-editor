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

  console.log("Preparing KaTeX CSS bundle with web fonts...");
  const katexCssPath = path.resolve(
    import.meta.dirname,
    "../node_modules/katex/dist/katex.min.css",
  );
  if (fs.existsSync(katexCssPath)) {
    let cssContent = fs.readFileSync(katexCssPath, "utf-8");
    // Point relative font URLs to CDN so the extension works standalone without bundled TTF files
    cssContent = cssContent.replace(
      /url\(fonts\//g,
      "url(https://cdn.jsdelivr.net/npm/katex@0.18.6/dist/fonts/",
    );
    fs.writeFileSync(path.join(outdir, "katex.css"), cssContent, "utf-8");
  }

  console.log("Extension bundles built successfully in extensions/dist/");
}

buildExtensions().catch((err) => {
  console.error("Build failed:", err);
  process.exit(1);
});
