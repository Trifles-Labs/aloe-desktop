import React, { useState } from "react";
import { Laptop, LogOut, UserRound } from "lucide-react";

import Button from "@aloe/ui/components/ui/Button";
import Modal from "@aloe/ui/components/ui/Modal";
import type { AgentConfig } from "../types";
import { ControlGroup, ControlRow } from "./ControlGroup";

type Props = { config: AgentConfig; onReset: () => void };

export function ConnectionPanel({ config, onReset }: Props) {
  const [confirming, setConfirming] = useState(false);

  return (
    <>
      <ControlGroup
        label="Account"
        action={
          <Button variant="ghost" size="sm" className="press-tap -mb-1 text-xs" onClick={() => setConfirming(true)}>
            <LogOut className="h-3.5 w-3.5" />
            Sign out
          </Button>
        }
        footnote={config.socketError ? `Last connection error: ${config.socketError}` : undefined}
      >
        {config.userProfile ? (
          <ControlRow icon={UserRound} title={config.userProfile.name || "Signed in"} detail={config.userProfile.email} />
        ) : null}
        <ControlRow icon={Laptop} title={config.deviceName} detail={config.platform} />
      </ControlGroup>

      {/* Signing out also drops this computer's folder grants from Aloe's reach until the
          next sign-in, which is the part worth a confirmation. */}
      <Modal open={confirming} onClose={() => setConfirming(false)} size="sm" labelledBy="logout-title">
        <h2 id="logout-title" className="text-lg font-semibold tracking-[-0.01em] text-ink">
          Sign out of Aloe?
        </h2>
        <p className="mt-2 text-[13px] leading-6 text-ink-soft">
          Aloe stops reaching this computer — no folder access, no commands, no local search — until you sign in again.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={() => {
              setConfirming(false);
              onReset();
            }}
          >
            Sign out
          </Button>
        </div>
      </Modal>
    </>
  );
}
