import { describe, it, expect } from "bun:test";
import { Schema } from "@milkdown/kit/prose/model";
import { decorate } from "../src/components/Editor/githubAlerts";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { content: "text*", group: "block" },
    blockquote: { content: "block+", group: "block" },
    text: {},
  },
});

const quote = (text: string) =>
  schema.node("doc", null, [
    schema.node("blockquote", null, [schema.node("paragraph", null, [schema.text(text)])]),
  ]);

describe("github alerts", () => {
  it("decorates a marked blockquote and hides only the marker", () => {
    const doc = quote("[!NOTE] hello");
    const decos = decorate(doc).find();
    expect(decos.length).toBe(3);
    const inline = decos.find((d) => d.from !== d.to && d.to - d.from < doc.nodeSize - 2)!;
    // "[!NOTE] " starts right inside blockquote+paragraph (pos 2) and is 8 chars.
    expect([inline.from, inline.to]).toEqual([2, 10]);
  });

  it("ignores plain blockquotes and unknown markers", () => {
    expect(decorate(quote("just a quote")).find().length).toBe(0);
    expect(decorate(quote("[!SHOUT] hi")).find().length).toBe(0);
  });
});
