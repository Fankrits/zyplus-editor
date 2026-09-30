// Generates a big project folder to open in the app: notes in nested folders, plus the
// kind of dependency trees a real project folder drags along.
//   node scripts/perf/fixture.mjs <dir> [notes=4000]
import fs from "node:fs";
import path from "node:path";

const dir = path.resolve(process.argv[2] ?? "");
const notes = Number(process.argv[3] ?? 4000);
if (!process.argv[2]) {
  console.error("usage: node scripts/perf/fixture.mjs <dir> [notes]");
  process.exit(1);
}

let files = 0;
function put(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
  files++;
}

// Notes: 20 topics x 10 sub-folders, spread evenly.
for (let i = 0; i < notes; i++) {
  const topic = `topic-${String(i % 20).padStart(2, "0")}`;
  const sub = `part-${String(Math.floor(i / 20) % 10)}`;
  put(path.join(dir, "notes", topic, sub, `note-${i}.md`), `# Note ${i}\n\nSome text for note ${i}.\n`);
}

// A dependency tree and a build output, each many times the size of the notes.
for (let p = 0; p < 300; p++) {
  for (let f = 0; f < 20; f++) put(path.join(dir, "node_modules", `pkg-${p}`, "lib", `file-${f}.js`), "module.exports = 1;\n");
}
for (let f = 0; f < 3000; f++) put(path.join(dir, "target", "debug", `dep-${Math.floor(f / 50)}`, `obj-${f}.o`), "\0");
put(path.join(dir, "README.md"), "# Fixture\n");

console.log(`${files} files under ${dir}`);
