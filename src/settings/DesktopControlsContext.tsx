/* The Desktop app settings pane is rendered by the shared Settings page, which
   only knows the slot's render function. The live agent config and the handlers
   that change it live in main.tsx, so they reach the pane through here. */

import { createContext, useContext } from "react";

import type { AgentConfig, CommandTrustMode, PendingApproval } from "../types";

export type DesktopControlsValue = {
  config: AgentConfig;
  pending: PendingApproval[];
  setConfig: (config: AgentConfig) => void;
  onRefresh: () => void;
  onReset: () => void;
  onAddFolder: () => void;
  onRemoveFolder: (path: string) => void;
  onSetCommandTrustMode: (mode: CommandTrustMode) => void;
  onSetDesktopControl: (enabled: boolean) => void;
};

const DesktopControlsContext = createContext<DesktopControlsValue | null>(null);

export const DesktopControlsProvider = DesktopControlsContext.Provider;

export function useDesktopControls(): DesktopControlsValue {
  const value = useContext(DesktopControlsContext);
  if (!value) throw new Error("useDesktopControls must be used inside DesktopControlsProvider.");
  return value;
}
