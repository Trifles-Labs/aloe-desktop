import React from "react";

type Props = { style?: React.CSSProperties };

export function ButterflyDecor({ style }: Props) {
  return (
    <svg
      viewBox="0 0 80 56"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={style}
      aria-hidden
    >
      <path d="M40 28 C30 10, 4 6, 6 20 C8 34, 32 30, 40 28Z" fill="currentColor" />
      <path d="M40 28 C50 10, 76 6, 74 20 C72 34, 48 30, 40 28Z" fill="currentColor" />
      <path d="M40 28 C33 38, 12 44, 14 34 C16 24, 36 30, 40 28Z" fill="currentColor" opacity="0.7" />
      <path d="M40 28 C47 38, 68 44, 66 34 C64 24, 44 30, 40 28Z" fill="currentColor" opacity="0.7" />
      <line x1="40" y1="22" x2="40" y2="34" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}
