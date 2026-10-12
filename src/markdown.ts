// A small Markdown renderer for the quick ask answers. It builds DOM nodes
// and never parses the model's text as HTML, so whatever an answer contains
// stays text. It is called again for every streamed piece, so a half-written
// answer renders too: an unclosed code fence is a code block so far, an
// unclosed **bold** shows its stars until the closing ones arrive.
//
// Supported: paragraphs (single line breaks kept), headings, lists (nested by
// indentation), block quotes, fenced code, tables, rules, and inline code,
// bold, italic, strikethrough and links (shown as text with the address, not
// followed).

const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const LIST_ITEM = /^(\s*)([-*+•]|\d{1,3}[.)])\s+(.*)$/;
const TABLE_DIVIDER = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

export function renderMarkdown(text: string): DocumentFragment {
  const fragment = document.createDocumentFragment();
  renderBlocks(text.replace(/\r\n?/g, "\n").split("\n"), fragment);
  return fragment;
}

function isBlockStart(line: string): boolean {
  return FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line) || LIST_ITEM.test(line);
}

function renderBlocks(lines: string[], parent: Node) {
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    if (!line.trim()) {
      i++;
      continue;
    }

    const fence = line.match(FENCE);
    if (fence) {
      const marker = fence[1];
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(marker)) {
        code.push(lines[i]);
        i++;
      }
      i++; // the closing fence, if it has arrived
      const pre = el("pre");
      const codeEl = el("code");
      codeEl.textContent = code.join("\n");
      pre.append(codeEl);
      parent.appendChild(pre);
      continue;
    }

    const heading = line.match(HEADING);
    if (heading) {
      const node = el(heading[1].length <= 2 ? "h3" : "h4");
      renderInline(heading[2], node);
      parent.appendChild(node);
      i++;
      continue;
    }

    if (RULE.test(line)) {
      parent.appendChild(el("hr"));
      i++;
      continue;
    }

    if (QUOTE.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i])) {
        quoted.push(lines[i].match(QUOTE)![1]);
        i++;
      }
      const blockquote = el("blockquote");
      renderBlocks(quoted, blockquote);
      parent.appendChild(blockquote);
      continue;
    }

    if (LIST_ITEM.test(line)) {
      i = renderList(lines, i, parent);
      continue;
    }

    if (line.includes("|") && i + 1 < lines.length && TABLE_DIVIDER.test(lines[i + 1])) {
      i = renderTable(lines, i, parent);
      continue;
    }

    // A paragraph: up to a blank line or the start of another block
    const paragraph: string[] = [];
    while (i < lines.length && lines[i].trim() && (paragraph.length === 0 || !isBlockStart(lines[i]))) {
      paragraph.push(lines[i].trim());
      i++;
    }
    const p = el("p");
    paragraph.forEach((text, index) => {
      if (index > 0) p.appendChild(el("br"));
      renderInline(text, p);
    });
    parent.appendChild(p);
  }
}

/** A list from line `start`; returns the first line after it. */
function renderList(lines: string[], start: number, parent: Node): number {
  const first = lines[start].match(LIST_ITEM)!;
  const indent = first[1].length;
  const ordered = /\d/.test(first[2]);
  const list = el(ordered ? "ol" : "ul");
  if (ordered) {
    const number = parseInt(first[2], 10);
    if (number > 1) (list as HTMLOListElement).start = number;
  }

  let i = start;
  while (i < lines.length) {
    const match = lines[i].match(LIST_ITEM);
    if (!match || match[1].length < indent) break;
    if (match[1].length > indent) {
      // A nested list belongs to the item before it
      const host = list.lastElementChild ?? list;
      i = renderList(lines, i, host);
      continue;
    }
    if (/\d/.test(match[2]) !== ordered) break;

    const item = el("li");
    const body: string[] = [match[3]];
    i++;
    // Continuation lines, indented under the item
    while (i < lines.length && lines[i].trim() && !LIST_ITEM.test(lines[i]) && /^\s+/.test(lines[i])) {
      body.push(lines[i].trim());
      i++;
    }
    body.forEach((text, index) => {
      if (index > 0) item.appendChild(el("br"));
      renderInline(text, item);
    });
    list.appendChild(item);

    // A blank line inside the list, if the list goes on after it
    if (i < lines.length && !lines[i].trim()) {
      let next = i;
      while (next < lines.length && !lines[next].trim()) next++;
      const after = next < lines.length ? lines[next].match(LIST_ITEM) : null;
      if (after && after[1].length >= indent) {
        i = next;
      } else {
        break;
      }
    }
  }
  parent.appendChild(list);
  return i;
}

function tableCells(line: string): string[] {
  let row = line.trim();
  if (row.startsWith("|")) row = row.slice(1);
  if (row.endsWith("|")) row = row.slice(0, -1);
  return row.split("|").map((cell) => cell.trim());
}

/** A table from line `start` (the header); returns the first line after it. */
function renderTable(lines: string[], start: number, parent: Node): number {
  const wrapper = el("div", "table");
  const table = el("table");
  const head = el("thead");
  const headRow = el("tr");
  for (const cell of tableCells(lines[start])) {
    const th = el("th");
    renderInline(cell, th);
    headRow.appendChild(th);
  }
  head.appendChild(headRow);
  table.appendChild(head);

  const body = el("tbody");
  let i = start + 2;
  while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
    const row = el("tr");
    for (const cell of tableCells(lines[i])) {
      const td = el("td");
      renderInline(cell, td);
      row.appendChild(td);
    }
    body.appendChild(row);
    i++;
  }
  table.appendChild(body);
  wrapper.appendChild(table);
  parent.appendChild(wrapper);
  return i;
}

/** Index of `marker` closing an inline span that starts at `from`, or -1. */
function closing(text: string, marker: string, from: number): number {
  let index = text.indexOf(marker, from);
  while (index !== -1) {
    // The closing marker follows text, not a space
    if (index > from && text[index - 1] !== " ") return index;
    index = text.indexOf(marker, index + 1);
  }
  return -1;
}

function renderInline(text: string, parent: Node) {
  let plain = "";
  const flush = () => {
    if (plain) parent.appendChild(document.createTextNode(plain));
    plain = "";
  };

  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    const next = text[i + 1] ?? "";

    // An escaped character is plain text
    if (ch === "\\" && /[\\`*_~[\]()#>|.!-]/.test(next)) {
      plain += next;
      i += 2;
      continue;
    }

    if (ch === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i + 1) {
        flush();
        const code = el("code");
        code.textContent = text.slice(i + 1, end);
        parent.appendChild(code);
        i = end + 1;
        continue;
      }
    }

    const double = text.slice(i, i + 2);
    if ((double === "**" || double === "__" || double === "~~") && text[i + 2] && text[i + 2] !== " ") {
      const end = closing(text, double, i + 2);
      if (end !== -1) {
        flush();
        const node = el(double === "~~" ? "del" : "strong");
        renderInline(text.slice(i + 2, end), node);
        parent.appendChild(node);
        i = end + 2;
        continue;
      }
    }

    // _ inside a word (snake_case) is no emphasis
    const wordBefore = i > 0 && /[\p{L}\p{N}]/u.test(text[i - 1]);
    if ((ch === "*" || (ch === "_" && !wordBefore)) && next && next !== " " && next !== ch) {
      const end = closing(text, ch, i + 1);
      const wordAfter = end !== -1 && /[\p{L}\p{N}]/u.test(text[end + 1] ?? "");
      if (end !== -1 && !(ch === "_" && wordAfter)) {
        flush();
        const em = el("em");
        renderInline(text.slice(i + 1, end), em);
        parent.appendChild(em);
        i = end + 1;
        continue;
      }
    }

    if (ch === "[") {
      const link = text.slice(i).match(/^\[([^\]]+)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/);
      if (link) {
        flush();
        const a = el("span", "link");
        renderInline(link[1], a);
        parent.appendChild(a);
        if (link[1].trim() !== link[2]) {
          const address = el("span", "link-address");
          address.textContent = ` (${link[2]})`;
          parent.appendChild(address);
        }
        i += link[0].length;
        continue;
      }
    }

    plain += ch;
    i++;
  }
  flush();
}
