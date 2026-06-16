import React from "react";
import { Check } from "lucide-react";

type Props = {
  setupToken: string;
  onTokenChange: (value: string) => void;
  onConnect: () => void;
};

export function AuthScreen({ setupToken, onTokenChange, onConnect }: Props) {
  return (
    <section className="auth-screen">
      <div className="panel auth-panel">
        <div>
          <p className="eyebrow" style={{ marginBottom: 6 }}>Get started</p>
          <h2 style={{ fontSize: 20, marginBottom: 8 }}>Connect to Aloe</h2>
          <p className="text-muted">
            Paste your setup token from the Aloe Integrations page to register this device.
          </p>
        </div>
        <label className="field-label" style={{ marginTop: 8 }}>
          Setup token
          <textarea
            value={setupToken}
            onChange={(e) => onTokenChange(e.target.value)}
            placeholder="Paste the setup token from Aloe Integrations…"
            rows={5}
          />
        </label>
        <div className="button-row" style={{ marginTop: 4 }}>
          <button
            className="primary"
            onClick={onConnect}
            disabled={!setupToken.trim()}
          >
            <Check size={15} />
            Log in with setup token
          </button>
        </div>
      </div>
    </section>
  );
}
