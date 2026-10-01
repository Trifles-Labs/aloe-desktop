import React from "react";
import { MousePointerClick, ShieldAlert } from "lucide-react";

import { ControlGroup } from "./ControlGroup";

type Props = {
  enabled: boolean;
  onSetEnabled: (enabled: boolean) => void;
};

/* Off by default, and only ever switched on from here — nothing the backend
   sends can flip it. The failsafe (pointer into a screen corner) switches it
   back off, which the 1.5s config poll then reflects in this row. */

export function DesktopControlPanel({ enabled, onSetEnabled }: Props) {
  return (
    <ControlGroup
      label="Desktop control"
      footnote={
        enabled ? (
          <span className="flex items-start gap-1.5 text-gold">
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            Aloe can click and type in any app you're signed into, including your browser. A green glow around your screen shows when it's working. To stop it, click Stop at the top of the screen or move the pointer into any corner.
          </span>
        ) : undefined
      }
    >
      <label className="settings-row cursor-pointer items-start">
        <span className="flex min-w-0 items-start gap-3">
          <MousePointerClick className="mt-0.5 h-4 w-4 shrink-0 text-moss" />
          <span className="min-w-0">
            <span className="block text-[13px] font-medium leading-5 text-ink">Let Aloe control this computer</span>
            <span className="mt-0.5 block text-xs leading-5 text-ink-soft">
              Aloe can see your main screen and use the mouse and keyboard to finish tasks in other apps. It can't click inside this window.
            </span>
          </span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="peer sr-only"
          checked={enabled}
          onChange={(event) => onSetEnabled(event.target.checked)}
        />
        <span
          aria-hidden
          className={`relative mt-0.5 inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors duration-150 peer-focus-visible:ring-2 peer-focus-visible:ring-moss/40 ${
            enabled ? "border-moss bg-moss" : "border-edge bg-surface-strong"
          }`}
        >
          <span
            className={`h-3.5 w-3.5 rounded-full shadow-sm transition-transform duration-150 ${
              enabled ? "translate-x-[18px] bg-cream" : "translate-x-[2px] bg-ink-soft/50"
            }`}
          />
        </span>
      </label>
    </ControlGroup>
  );
}
