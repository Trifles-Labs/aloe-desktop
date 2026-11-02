import katex from "katex";
import "katex/dist/katex.min.css";
import "@fontsource/caveat/500.css";
import "@fontsource/caveat/700.css";

/* Aloe's handwriting over the screen (a draw_on_screen "write" mark, drawn by voice-drawing.ts).
   A small Markdown subset, one line per line: # headings, **bold**, *italic*, `code`, - and 1.
   lists, with LaTeX in $…$ or \(…\) inline and $$…$$ or \[…\] on a line of its own, set by KaTeX
   with trust off (so \href, \url and \html… do nothing). Built from DOM nodes and textContent;
   nothing Aloe sends is parsed as HTML. Every word and formula is its own piece, and each is
   written out after the last at a pace set by its length (--d and --t, used by voice.html). */

const MS_PER_CHAR = 34;
const MS_PER_TEX_CHAR = 20;
const WORD_GAP_MS = 50;
const LINE_GAP_MS = 180;
/** However much is written at once, it's all there by then. */
const MAX_MS = 8000;

type Piece = { element: HTMLElement; ms: number; pause: number };

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
   just after), `code`, **bold**, *italic*. */
const INLINE = /\\\((.+?)\\\)|\$\$(.+?)\$\$|\$(\S(?:[^$\n]*?\S)?)\$(?!\d)|`([^`\n]+)`|\*\*(.+?)\*\*|\*(\S(?:[^*\n]*?\S)?)\*/g;

const inline = (text: string, into: HTMLElement, pieces: Piece[]) => {
  let last = 0;
  for (const match of text.matchAll(INLINE)) {
    words(text.slice(last, match.index), into, pieces);
    const [, paren, doubled, single, code, bold, italic] = match;
    const tex = paren ?? doubled ?? single;
    if (tex !== undefined) into.append(math(tex, false, pieces));
    else if (code !== undefined) {
      const node = element("code", "wd", code);
      into.append(node);
      pieces.push({ element: node, ms: Math.max(90, code.length * MS_PER_CHAR), pause: WORD_GAP_MS });
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

/** The writing as nodes, and how long it takes to write out from its start. */
export function write(markdown: string): { element: HTMLElement; duration: number } {
  const root = element("div", "write-body");
  const pieces: Piece[] = [];
  const lines = markdown.split("\n");
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i]!.trim();
    if (!line) {
      root.append(element("div", "gap"));
      continue;
    }
    const display = displayMath(lines, i);
    if (display) {
      if (display.tex) root.append(math(display.tex, true, pieces));
      i = display.end;
      continue;
    }
    const block = element("div", "line");
    const heading = line.match(/^(#{1,3})\s+(.*)$/);
    const listed = line.match(/^(?:([-*•])|(\d{1,3})[.)])\s+(.*)$/);
    if (heading) {
      block.classList.add(`h${heading[1]!.length}`);
      line = heading[2]!;
    } else if (listed) {
      block.classList.add("item");
      const bullet = element("span", "wd bullet", listed[2] ? `${listed[2]}.` : "–");
      block.append(bullet);
      pieces.push({ element: bullet, ms: 120, pause: WORD_GAP_MS });
      line = listed[3]!;
    }
    inline(line, block, pieces);
    root.append(block);
    if (pieces.length) pieces[pieces.length - 1]!.pause = LINE_GAP_MS;
  }

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
