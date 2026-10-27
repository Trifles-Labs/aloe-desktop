/* The Files pane's editor: CodeMirror 6, themed from Aloe's CSS variables so it follows light and
 * dark mode without a second theme. Remount it (change its key) to load a different document. */

import { useEffect, useRef } from "react";
import { basicSetup } from "codemirror";
import { Compartment, EditorState, Prec, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";

type Props = {
  path: string;
  initialValue: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onSave: () => void;
};

export function languageFor(path: string): Extension | null {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  switch (extension) {
    case "js":
    case "mjs":
    case "cjs":
      return javascript();
    case "jsx":
      return javascript({ jsx: true });
    case "ts":
    case "mts":
    case "cts":
      return javascript({ typescript: true });
    case "tsx":
      return javascript({ jsx: true, typescript: true });
    case "py":
      return python();
    case "rs":
      return rust();
    case "json":
    case "jsonc":
      return json();
    case "md":
    case "mdx":
      return markdown();
    case "css":
      return css();
    case "html":
    case "htm":
    case "svelte":
    case "vue":
      return html();
    default:
      return null;
  }
}

const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--surface)", color: "var(--foreground)", fontSize: "12px" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono), ui-monospace, monospace", lineHeight: "1.55" },
  ".cm-gutters": { backgroundColor: "var(--surface)", color: "var(--ink-soft)", borderRight: "1px solid var(--edge)" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "color-mix(in srgb, var(--sage-soft) 70%, transparent)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
    backgroundColor: "color-mix(in srgb, var(--sky) 28%, transparent) !important",
  },
  ".cm-panels": { backgroundColor: "var(--surface-strong)", color: "var(--foreground)", borderColor: "var(--edge)" },
  ".cm-tooltip": { backgroundColor: "var(--surface-strong)", border: "1px solid var(--edge)" },
  ".cm-matchingBracket": { backgroundColor: "color-mix(in srgb, var(--moss) 20%, transparent)" },
});

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier, tags.controlKeyword, tags.operatorKeyword], color: "var(--moss)", fontWeight: "600" },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: "var(--clay)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--gold)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--ink-soft)", fontStyle: "italic" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.definition(tags.function(tags.variableName))], color: "var(--sky)" },
  { tag: [tags.typeName, tags.className, tags.namespace], color: "var(--gold)" },
  { tag: [tags.tagName, tags.heading], color: "var(--moss)", fontWeight: "600" },
  { tag: [tags.attributeName, tags.propertyName], color: "var(--foreground)" },
  { tag: tags.link, color: "var(--sky)", textDecoration: "underline" },
  { tag: tags.invalid, color: "var(--danger)" },
]);

export default function CodeEditor({ path, initialValue, readOnly, onChange, onSave }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const editable = useRef(new Compartment());
  const callbacks = useRef({ onChange, onSave });
  callbacks.current = { onChange, onSave };

  useEffect(() => {
    if (!host.current) return;
    const language = languageFor(path);
    const created = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: initialValue,
        extensions: [
          basicSetup,
          theme,
          syntaxHighlighting(highlight),
          language ?? [],
          editable.current.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
          Prec.highest(keymap.of([{ key: "Mod-s", preventDefault: true, run: () => (callbacks.current.onSave(), true) }])),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) callbacks.current.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = created;
    return () => {
      created.destroy();
      view.current = null;
    };
    // The document is loaded once per mount; the parent remounts with a new key to load another.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    view.current?.dispatch({ effects: editable.current.reconfigure([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]) });
    if (!readOnly) view.current?.focus();
  }, [readOnly]);

  return <div ref={host} className="h-full min-h-0 overflow-hidden" />;
}
