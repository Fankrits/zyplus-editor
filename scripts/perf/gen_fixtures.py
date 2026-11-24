#!/usr/bin/env python3
"""Generates the markdown fixtures the perf harness opens. Usage: gen_fixtures.py <outdir>"""
import pathlib, random, sys

out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
random.seed(1)
WORDS = "the quick brown fox jumps over lazy dog running shoes apparel brand editor markdown rich text".split()


def para(n=60):
    return " ".join(random.choice(WORDS) for _ in range(n)) + "\n\n"


doc = "# Typical note\n\n"
for i in range(40):
    doc += f"## Section {i}\n\n" + para() + "- item one\n- item two **bold** and `code`\n\n"
    if i % 8 == 0:
        doc += "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n"
    if i % 10 == 0:
        doc += "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n\n"
doc += "```mermaid\ngraph TD; A-->B; B-->C;\n```\n\n```latex\nE = mc^2 + \\int_0^1 x^2 dx\n```\n"
(out / "typical.md").write_text(doc)
(out / "big.md").write_text("# Big\n\n" + "".join(f"## S{i}\n\n" + para(80) for i in range(6000)))
for i in range(10):
    (out / f"tab{i}.md").write_text(f"# Tab {i}\n\n" + para(200) * 20)
