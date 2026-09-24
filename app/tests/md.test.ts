// Run with: npm test (node --test, no extra dependencies).
import assert from "node:assert/strict";
import { test } from "node:test";
import { inline, render } from "../src/lib/md.ts";

test("raw HTML from agents is shown, never executed", () => {
  const evil = [
    `<img src=x onerror="alert(1)">`,
    `<script>alert(1)</script>`,
    "`</code><img src=x onerror=alert(1)>`",
    "**<svg onload=alert(1)>**",
    "```\n</pre><script>alert(1)</script>\n```",
    `[click](javascript:alert(1))`,
    `" onmouseover="alert(1)`,
  ];
  for (const s of evil) {
    for (const out of [render(s), inline(s)]) {
      assert.ok(!/<(img|script|svg|a)\b/i.test(out), `${s} -> ${out}`);
      assert.ok(!/\son\w+=/i.test(out.replace(/&quot;|&#39;/g, "")) || !/<[^>]+\son\w+=/i.test(out), `${s} -> ${out}`);
    }
  }
});

test("the few allowed constructs render", () => {
  assert.equal(inline("run `cargo test` **now**"), "run <code>cargo test</code> <strong>now</strong>");
  const html = render("# Title\n\nA para\ncontinues.\n\n- one\n- two\n\n```\ncode <b>\n```");
  assert.match(html, /<h3>Title<\/h3>/);
  assert.match(html, /<p>A para continues\.<\/p>/);
  assert.match(html, /<ul><li>one<\/li><li>two<\/li><\/ul>/);
  assert.match(html, /<pre><code>code &lt;b&gt;<\/code><\/pre>/);
});
