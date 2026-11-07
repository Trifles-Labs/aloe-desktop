import katex from "katex";
import "katex/dist/katex.min.css";
import "@fontsource/caveat/500.css";
import "@fontsource/caveat/700.css";

/* Aloe's handwriting over the screen (a draw_on_screen "write" mark, drawn by voice-drawing.ts),
   and the voice answer on the island (voice-overlay.ts), from one parser. A small Markdown subset,
   one line per line: # headings, **bold**, *italic*, `code`, ``` fences, [links](…) as their text,
   - and 1. lists, with LaTeX in $…$ or \(…\) inline and $$…$$ or \[…\] on a line of its own, set
   by KaTeX with trust off (so \href, \url and \html… do nothing). Built from DOM nodes and
   textContent; nothing Aloe sends is parsed as HTML. Every word is its own .wd span (the island's
   read-along lights them up). Handwriting writes each word and formula out after the last at a
   pace set by its length (--d and --t, used by voice.html). */

const MS_PER_CHAR = 34;
const MS_PER_TEX_CHAR = 20;
const WORD_GAP_MS = 50;
const LINE_GAP_MS = 180;
/** However much is written at once, it's all there by then. */
const MAX_MS = 8000;

type Piece = { element: HTMLElement; ms: number; pause: number };
/** One line, list item, code fence or display formula, and the source text it came from. */
export type Block = { source: string; element: HTMLElement };

const clamp = (n: number, low: number, high: number) => Math.min(high, Math.max(low, n));

const element = (tag: string, className: string, text?: string) => {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};

const math = (tex: string, display: boolean, pieces: Piece[]) => {
  const node = element(display ? "div" : "span", display ? "wd tex display" : "wd tex");
  try {
    katex.render(tex, node, { displayMode: display, throwOnError: false, trust: false, strict: "ignore", output: "html", maxSize: 20, maxExpand: 300 });
  } catch {
    node.textContent = tex;
  }
  pieces.push({ element: node, ms: clamp(tex.length * MS_PER_TEX_CHAR, 260, 2400), pause: display ? LINE_GAP_MS : WORD_GAP_MS });
  return node;
};

const words = (text: string, into: HTMLElement, pieces: Piece[]) => {
  for (const part of text.split(/(\s+)/)) {
    if (!part) continue;
    if (/^\s+$/.test(part)) {
      into.append(" ");
      continue;
    }
    const word = element("span", "wd", part);
    into.append(word);
    pieces.push({ element: word, ms: Math.max(90, part.length * MS_PER_CHAR), pause: WORD_GAP_MS });
  }
};

/* \(…\), $$…$$ and $…$ maths (a $ amount like "$5 and $10" isn't: no space just inside, no digit
   just after), `code`, **bold**, *italic*, [text](link) as its text. */
const INLINE = /\\\((.+?)\\\)|\$\$(.+?)\$\$|\$(\S(?:[^$\n]*?\S)?)\$(?!\d)|`([^`\n]+)`|\*\*(.+?)\*\*|\*(\S(?:[^*\n]*?\S)?)\*|!?\[([^\]\n]+)\]\([^)\n]*\)/g;

const inline = (text: string, into: HTMLElement, pieces: Piece[]) => {
  let last = 0;
  for (const match of text.matchAll(INLINE)) {
    words(text.slice(last, match.index), into, pieces);
    const [, paren, doubled, single, code, bold, italic, link] = match;
    const tex = paren ?? doubled ?? single;
    if (tex !== undefined) into.append(math(tex, false, pieces));
    else if (code !== undefined) {
      const node = element("code", "wd", code);
      into.append(node);
      pieces.push({ element: node, ms: Math.max(90, code.length * MS_PER_CHAR), pause: WORD_GAP_MS });
    } else if (link !== undefined) {
      inline(link, into, pieces);
    } else {
      const node = element(bold !== undefined ? "strong" : "em", "");
      into.append(node);
      inline(bold ?? italic ?? "", node, pieces);
    }
    last = match.index + match[0].length;
  }
  words(text.slice(last), into, pieces);
};

/* Display maths starting on line `from` ($$…$$ or \[…\], perhaps over several lines): its TeX and
   the line it ends on, or null when the line doesn't open one. */
const displayMath = (lines: string[], from: number): { tex: string; end: number } | null => {
  const first = lines[from]!.trim();
  const close = first.startsWith("$$") ? "$$" : first.startsWith("\\[") ? "\\]" : null;
  if (!close) return null;
  let rest = first.slice(2);
  for (let i = from; i < lines.length; i++) {
    if (i > from) rest += `\n${lines[i]!.trim()}`;
    const at = rest.indexOf(close);
    if (at >= 0) return { tex: rest.slice(0, at).trim(), end: i };
  }
  // Never closed: the rest of it is the formula.
  return { tex: rest.trim(), end: lines.length - 1 };
};

/* A ``` fence starting on line `from`: its code and the line it ends on (the rest, if unclosed). */
const fence = (lines: string[], from: number): { code: string; end: number } | null => {
  if (!lines[from]!.trim().startsWith("```")) return null;
  for (let i = from + 1; i < lines.length; i++) {
    if (lines[i]!.trim().startsWith("```")) return { code: lines.slice(from + 1, i).join("\n"), end: i };
  }
  return { code: lines.slice(from + 1).join("\n"), end: lines.length - 1 };
};

function parse(markdown: string, pieces: Piece[]): Block[] {
  const blocks: Block[] = [];
  const lines = markdown.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const from = i;
    const source = () => lines.slice(from, i + 1).join("\n");
    let line = lines[i]!.trim();
    if (!line) {
      blocks.push({ source: "", element: element("div", "gap") });
      continue;
    }
    const code = fence(lines, i);
    if (code) {
      const node = element("pre", "code", code.code);
      pieces.push({ element: node, ms: clamp(code.code.length * MS_PER_TEX_CHAR, 260, 2400), pause: LINE_GAP_MS });
      i = code.end;
      blocks.push({ source: source(), element: node });
      continue;
    }
    const display = displayMath(lines, i);
    if (display) {
      i = display.end;
      if (display.tex) blocks.push({ source: source(), element: math(display.tex, true, pieces) });
      continue;
    }
    const block = element("div", "line");
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    const listed = line.match(/^(?:([-*•+])|(\d{1,3})[.)])\s+(.*)$/);
    if (heading) {
      block.classList.add(`h${Math.min(heading[1]!.length, 3)}`);
      line = heading[2]!;
    } else if (listed) {
      block.classList.add("item");
      const bullet = element("span", "wd bullet", listed[2] ? `${listed[2]}.` : "–");
      block.append(bullet);
      pieces.push({ element: bullet, ms: 120, pause: WORD_GAP_MS });
      line = listed[3]!;
    }
    inline(line, block, pieces);
    blocks.push({ source: source(), element: block });
    if (pieces.length) pieces[pieces.length - 1]!.pause = LINE_GAP_MS;
  }
  return blocks;
}

/** Markdown as blocks of nodes, with no handwriting timing: the voice answer on the island. A
 * formula there isn't a word the read-along can match against speech, so it isn't a .wd. */
export function markdownBlocks(markdown: string): Block[] {
  const blocks = parse(markdown, []);
  for (const { element: block } of blocks) {
    if (block.classList.contains("tex")) block.classList.remove("wd");
    for (const tex of block.querySelectorAll(".tex")) tex.classList.remove("wd");
  }
  return blocks;
}

/** The writing as nodes, and how long it takes to write out from its start. */
export function write(markdown: string): { element: HTMLElement; duration: number } {
  const root = element("div", "write-body");
  const pieces: Piece[] = [];
  root.append(...parse(markdown, pieces).map((block) => block.element));

  const total = pieces.reduce((sum, piece) => sum + piece.ms + piece.pause, 0);
  const scale = total > MAX_MS ? MAX_MS / total : 1;
  let at = 0;
  for (const piece of pieces) {
    piece.element.style.setProperty("--d", `${Math.round(at)}ms`);
    piece.element.style.setProperty("--t", `${Math.round(piece.ms * scale)}ms`);
    at += (piece.ms + piece.pause) * scale;
  }
  return { element: root, duration: at };
}
