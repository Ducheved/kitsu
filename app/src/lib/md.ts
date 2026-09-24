// A deliberately small Markdown renderer. Task bodies, decisions and agent
// messages are text written by people and by agents; rendered as HTML in a
// window that can call privileged commands, they are an injection surface.
// So: escape everything first, then allow a handful of constructs. No raw
// HTML, no links with javascript: URLs (no links at all, in fact), no images.

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

/** Inline-only rendering (code spans, bold, italics) for one-line texts. */
export function inline(s: string): string {
  // Code spans first so their content isn't touched by the other rules.
  const parts = s.split(/(`[^`]+`)/g);
  return parts
    .map((p) => {
      if (p.startsWith("`") && p.endsWith("`") && p.length > 1) return `<code>${esc(p.slice(1, -1))}</code>`;
      return esc(p)
        .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
        .replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
    })
    .join("");
}

export function render(md: string): string {
  const out: string[] = [];
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  let para: string[] = [];
  const flush = () => {
    if (para.length) out.push(`<p>${inline(para.join(" "))}</p>`);
    para = [];
  };
  while (i < lines.length) {
    const line = lines[i]!;
    if (line.startsWith("```")) {
      flush();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.startsWith("```")) code.push(lines[i++]!);
      i++;
      out.push(`<pre><code>${esc(code.join("\n"))}</code></pre>`);
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) {
      flush();
      const level = Math.min(h[1]!.length + 2, 6);
      out.push(`<h${level}>${inline(h[2]!)}</h${level}>`);
      i++;
      continue;
    }
    if (/^\s*[-*]\s+/.test(line)) {
      flush();
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i]!)) {
        let item = lines[i]!.replace(/^\s*[-*]\s+/, "");
        i++;
        while (i < lines.length && /^\s{2,}\S/.test(lines[i]!) && !/^\s*[-*]\s+/.test(lines[i]!)) item += " " + lines[i++]!.trim();
        items.push(`<li>${inline(item)}</li>`);
      }
      out.push(`<ul>${items.join("")}</ul>`);
      continue;
    }
    if (line.trim() === "") {
      flush();
      i++;
      continue;
    }
    para.push(line.trim());
    i++;
  }
  flush();
  return out.join("\n");
}
