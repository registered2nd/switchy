#!/usr/bin/env node
// Strip thinking and redacted_thinking content blocks from a Claude Code
// session JSONL.
//
// Claude Code splits a single assistant API response across multiple lines,
// one per content block, all sharing the same message.id. A thinking
// block lives on its own line. We DROP those lines entirely. Same for
// redacted_thinking — same provenance problem (invalid data when crossing
// upstream signing keys), same fix.
//
// IMPORTANT: lines are linked by parentUuid → uuid pointers. If we drop a
// line with uuid U whose parentUuid is P, we must rewrite every subsequent
// line's parentUuid from U to P, otherwise the conversation chain breaks
// and Claude Code can't reconstruct the resumed transcript past the gap.
//
// Backs up the original to <name>.bak.jsonl before writing.
import { readFileSync, writeFileSync, copyFileSync } from "node:fs";

const src = process.argv[2];
if (!src) {
  console.error("usage: node strip-thinking.mjs <session.jsonl>");
  process.exit(1);
}

const bak = src.replace(/\.jsonl$/, ".bak.jsonl");
copyFileSync(src, bak);
console.log(`backed up to ${bak}`);

const input = readFileSync(src, "utf8");
const rawLines = input.split(/\r?\n/);

const parsed = rawLines.map((line) => {
  if (!line.trim()) return { raw: line, obj: null };
  try {
    return { raw: line, obj: JSON.parse(line) };
  } catch {
    return { raw: line, obj: null };
  }
});

const remap = new Map(); // dropped uuid -> surviving parentUuid
let droppedLines = 0;
let strippedBlocks = 0;
const survivors = [];

for (const entry of parsed) {
  const { obj } = entry;
  if (!obj) {
    survivors.push(entry);
    continue;
  }
  const content = obj?.message?.content;
  if (Array.isArray(content)) {
    const thinkingCount = content.filter((b) => b?.type === "thinking" || b?.type === "redacted_thinking").length;
    if (thinkingCount > 0 && thinkingCount === content.length) {
      if (obj.uuid) {
        let parent = obj.parentUuid ?? null;
        while (parent && remap.has(parent)) parent = remap.get(parent);
        remap.set(obj.uuid, parent);
      }
      droppedLines++;
      strippedBlocks += thinkingCount;
      continue;
    }
    if (thinkingCount > 0) {
      obj.message.content = content.filter((b) => b?.type !== "thinking" && b?.type !== "redacted_thinking");
      strippedBlocks += thinkingCount;
    }
  }
  if (obj.parentUuid && remap.has(obj.parentUuid)) {
    obj.parentUuid = remap.get(obj.parentUuid);
  }
  survivors.push({ raw: JSON.stringify(obj), obj });
}

writeFileSync(src, survivors.map((e) => e.raw).join("\n"));
console.log(
  `dropped ${droppedLines} thinking-only line(s), stripped ${strippedBlocks} thinking block(s) total, rewrote ${remap.size} parentUuid chain(s)`,
);
