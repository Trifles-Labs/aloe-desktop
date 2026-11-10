import ChatWidgetRenderer from "@aloe/ui/components/chat/ChatWidgetRenderer";
import { FIGURE_EXAMPLES } from "@aloe/ui/components/chat/figures/examples";
import type { ChatWidget } from "@aloe/ui/lib/types";

const widgets: ChatWidget[] = FIGURE_EXAMPLES.map((example, index) => ({
  id: `figure-${index}`,
  kind: "figure",
  title: example.hard ? `${example.title} · hard` : example.title,
  figure: example.spec,
}));

/** The chat canvas with every figure the model can ask for. No sign-in: this is the renderer the thread uses. */
export function FiguresCanvas() {
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col px-4 py-8">
      <p className="text-[15px] leading-7 text-ink">
        A reply on the canvas. The model names the case. These are the figures it can put in the thread, labels in the same hand as the on-screen writing.
      </p>
      <ChatWidgetRenderer widgets={widgets} />
    </div>
  );
}
