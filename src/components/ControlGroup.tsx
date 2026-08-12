import React from "react";
import { motion, useReducedMotion } from "framer-motion";

import { GROUP_IN, GROUP_IN_REDUCED } from "@/lib/motion";
import { cn } from "@/lib/utils";

/* The same grouped-list vocabulary the web app's settings console uses — a mono
   label, one rounded sheet, hairline-separated rows — with room for an action
   beside the label. Four bordered cards each carrying their own display-scale
   heading is what this page used to be, and most of why it read as clutter.
   These declare entrance variants but never their own initial/animate, so they
   inherit the stagger from the page container. */

export function ControlGroup({
  label,
  action,
  footnote,
  className,
  children,
}: {
  label: string;
  action?: React.ReactNode;
  footnote?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.section variants={reduceMotion ? GROUP_IN_REDUCED : GROUP_IN} className="mt-6 first:mt-0">
      <div className="mb-2 flex items-end justify-between gap-3 px-1">
        <h2 className="spec-label">{label}</h2>
        {action}
      </div>
      <div className={cn("settings-group", className)}>{children}</div>
      {footnote && <p className="mt-2 px-1 text-xs leading-5 text-ink-soft">{footnote}</p>}
    </motion.section>
  );
}

/** A row that is only ever text — what the thing is, and what it currently says. */
export function ControlRow({
  icon: Icon,
  title,
  detail,
  control,
  className,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  title: React.ReactNode;
  detail?: React.ReactNode;
  control?: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("settings-row", className)}>
      <div className="flex min-w-0 items-start gap-3">
        {Icon && <Icon className="mt-0.5 h-4 w-4 shrink-0 text-moss" />}
        <div className="min-w-0">
          <p className="text-[13px] font-medium leading-5 text-ink">{title}</p>
          {detail && <div className="mt-0.5 text-xs leading-5 text-ink-soft">{detail}</div>}
        </div>
      </div>
      {control && <div className="flex shrink-0 items-center gap-2">{control}</div>}
    </div>
  );
}

/** Nothing here yet — said inside the sheet rather than as a fifth card. */
export function EmptyRow({ children }: { children: React.ReactNode }) {
  return <p className="px-4 py-8 text-center text-[13px] leading-5 text-ink-soft">{children}</p>;
}
