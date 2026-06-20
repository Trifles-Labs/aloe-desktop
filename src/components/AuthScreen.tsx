import React from "react";
import { Check, Leaf, ShieldCheck } from "lucide-react";

type Props = {
  setupToken: string;
  onTokenChange: (value: string) => void;
  onConnect: () => void;
};

export function AuthScreen({ setupToken, onTokenChange, onConnect }: Props) {
  return (
    <div className="flex min-h-full items-center justify-center px-6 py-10">
      <div className="liquid-glass w-full max-w-md rounded-3xl p-6 sm:p-8">
        <div className="flex items-center gap-3 border-b border-[#e1e7d7] pb-6 dark:border-[#2a3a28]">
          <div className="brand-mark h-11 w-11">
            <Leaf className="h-5 w-5" />
          </div>
          <div>
            <p className="eyebrow">Get started</p>
            <p className="mt-1 font-display text-lg font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Connect to Aloe</p>
          </div>
        </div>

        <p className="mt-6 text-sm leading-6 text-[#506257] dark:text-[#8aaa90]">
          Paste your setup token from the Aloe Integrations page to register this device.
        </p>

        <label className="mt-5 block">
          <span className="text-xs font-semibold uppercase tracking-wide text-[#506257] dark:text-[#8aaa90]">Setup token</span>
          <textarea
            value={setupToken}
            onChange={(e) => onTokenChange(e.target.value)}
            placeholder="Paste the setup token from Aloe Integrations…"
            rows={5}
            className="mt-2 w-full resize-none rounded-2xl border border-[#d9e0d5] bg-white/70 px-3.5 py-3 text-sm font-medium text-[#0b3026] outline-none transition-colors placeholder:text-[#8a9a84] focus:border-[#6f8747] focus:ring-2 focus:ring-[#c9d8b6] dark:border-[#2a3a28] dark:bg-[#1c2a20] dark:text-[#e8f0e0] dark:placeholder:text-[#6a8870] dark:focus:ring-[#2a3d22]"
          />
        </label>

        <button
          type="button"
          onClick={onConnect}
          disabled={!setupToken.trim()}
          className="primary-button mt-6 w-full disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Check className="h-4 w-4" />
          Log in with setup token
        </button>

        <div className="mt-6 flex items-center gap-2 rounded-2xl border border-[#dfe6d2] bg-[#dfe9d2]/70 px-4 py-3 text-xs text-[#506257] dark:border-[#2a3d22] dark:bg-[#2a3d22]/40 dark:text-[#8aaa90]">
          <ShieldCheck className="h-4 w-4 text-[#0b3026] dark:text-[#8faa5f]" />
          Your token stays on this device and is never shared.
        </div>
      </div>
    </div>
  );
}
