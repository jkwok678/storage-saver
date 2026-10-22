import {
  type AiProviders,
  findProvider,
  PROVIDER_IDS,
  providerDotClass,
  providerStatusText,
} from "../ai/aiProviders";
import { runningInApp } from "../shared/format";
import "./settings.css";

type Props = {
  ai: AiProviders;
};

function SettingsView({ ai }: Props) {
  return (
    <section className="settings-panel" aria-label="Settings">
      <div className="settings-scroll">
        <div className="settings-section">
          <div className="settings-section-head">
            <div>
              <h2>AI accounts</h2>
              <p>
                The AI Advisor uses your own Claude Code or Cursor subscription through its
                command-line tool on this Mac.
              </p>
            </div>
            <button
              type="button"
              className="settings-btn"
              onClick={ai.refresh}
              disabled={!runningInApp() || ai.busy !== null}
              title="Check the sign-in status again"
            >
              Refresh
            </button>
          </div>

          <ul className="settings-accounts">
            {PROVIDER_IDS.map((id) => {
              const provider = findProvider(ai, id);
              const isBusy = ai.busy === id;
              return (
                <li key={id} className="settings-account">
                  <span className={`status-dot ${providerDotClass(ai, provider)}`} />
                  <div className="settings-account-info">
                    <span className="settings-account-name">{provider.name}</span>
                    <span className="settings-account-status">
                      {providerStatusText(ai, provider)}
                    </span>
                  </div>
                  {!ai.checking &&
                    provider.installed &&
                    (provider.connected ? (
                      <button
                        type="button"
                        className="settings-btn"
                        onClick={() => ai.disconnect(id)}
                        disabled={ai.busy !== null}
                        title={`Signs ${provider.name} out on this Mac, including in the terminal`}
                      >
                        {isBusy ? "Signing out…" : "Sign out"}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="settings-btn primary"
                        onClick={() => ai.connect(id)}
                        disabled={ai.busy !== null}
                      >
                        {isBusy ? "Connecting…" : "Connect"}
                      </button>
                    ))}
                </li>
              );
            })}
          </ul>

          {ai.error && <p className="ai-error-banner">{ai.error}</p>}
        </div>
      </div>
    </section>
  );
}

export default SettingsView;
