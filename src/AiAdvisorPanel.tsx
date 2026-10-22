import { useEffect, useRef, useState } from "react";
import { CloseIcon, SparklesIcon } from "./icons";
import { runningInApp } from "./format";
import {
  findProvider,
  providerDotClass,
  providerStatusText,
  type AiProviderId,
  type AiProviders,
} from "./aiProviders";
import { ChatMessageRow } from "./ChatMessageRow";
import { ModelPickerRow } from "./ModelPickerRow";
import { useAiChat } from "./useAiChat";
import { useModelPicker } from "./useModelPicker";
import { useTrash } from "./useTrash";

type Props = {
  ai: AiProviders;
  currentPath: string;
  hidden?: boolean;
  onClose: () => void;
  onOpenSettings: () => void;
  onItemTrashed?: (path: string, freedBytes: number) => void;
};

const STARTER_PROMPTS = [
  "What can I safely delete right now to free up space?",
  "Analyze the current folder and tell me what is taking space",
  "Explain my largest files and what can be cleaned up",
  "Find developer caches, DerivedData, and build artifacts",
];

export function AiAdvisorPanel({
  ai,
  currentPath,
  hidden,
  onClose,
  onOpenSettings,
  onItemTrashed,
}: Props) {
  const [selectedProvider, setSelectedProvider] = useState<AiProviderId>("claude");
  const scrollRef = useRef<HTMLDivElement>(null);

  // On the first status only: a later refresh must not override the provider the user chose.
  const pickedProvider = useRef(false);
  useEffect(() => {
    const statuses = ai.providers;
    if (!statuses || pickedProvider.current) return;
    pickedProvider.current = true;
    setSelectedProvider((current) => {
      if (statuses.some((s) => s.id === current && s.connected)) return current;
      return statuses.find((s) => s.connected)?.id ?? current;
    });
  }, [ai.providers]);

  const activeProvider = findProvider(ai, selectedProvider);
  const picker = useModelPicker(selectedProvider, activeProvider);
  const chat = useAiChat({
    provider: selectedProvider,
    activeProvider,
    currentPath,
    modelChoice: picker.modelChoice,
    effortChoice: picker.effortChoice,
  });
  const trash = useTrash(onItemTrashed);
  const { messages, querying, statusMessage, queryError } = chat;

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, statusMessage, querying]);

  const statusText = runningInApp()
    ? providerStatusText(ai, activeProvider)
    : "Open Storage Saver to use the AI Advisor";

  return (
    <aside className="ai-panel" aria-label="AI Storage Advisor" hidden={hidden}>
      <header className="ai-panel-header">
        <div className="ai-panel-title">
          <span className="ai-sparkles-icon">
            <SparklesIcon />
          </span>
          <h2>AI Advisor</h2>
        </div>
        <button
          type="button"
          className="ai-icon-btn"
          onClick={onClose}
          aria-label="Close AI Advisor"
          title="Close AI Advisor"
        >
          <CloseIcon />
        </button>
      </header>

      <section className="ai-auth-section">
        <div className="ai-provider-toggle" role="tablist" aria-label="AI Provider">
          <button
            type="button"
            role="tab"
            aria-selected={selectedProvider === "claude"}
            className={selectedProvider === "claude" ? "active" : ""}
            onClick={() => setSelectedProvider("claude")}
          >
            Claude Code
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={selectedProvider === "cursor"}
            className={selectedProvider === "cursor" ? "active" : ""}
            onClick={() => setSelectedProvider("cursor")}
          >
            Cursor
          </button>
        </div>

        <div className="ai-auth-status">
          <div className="ai-status-indicator">
            <span className={`status-dot ${providerDotClass(ai, activeProvider)}`} />
            <span className="status-text">{statusText}</span>
          </div>

          <div className="ai-auth-actions">
            <button
              type="button"
              className="ai-text-btn"
              onClick={onOpenSettings}
              title="Connect or sign out of accounts in Settings"
            >
              {!ai.checking && !activeProvider.connected && activeProvider.installed
                ? "Connect in Settings"
                : "Manage accounts"}
            </button>
          </div>
        </div>
      </section>

      <div className="ai-chat-scroll" ref={scrollRef}>
        {messages.length === 0 ? (
          <div className="ai-empty-state">
            <div className="ai-empty-icon">
              <SparklesIcon />
            </div>
            <h3>Ask anything about your disk</h3>
            <p>
              Uses your {activeProvider.name} subscription. The AI queries
              your scanned folder tree, largest files, and inspects disk paths
              in read-only mode to find what can be safely deleted.
            </p>
            <div className="ai-starter-list">
              <span className="ai-starter-title">Suggested questions:</span>
              {STARTER_PROMPTS.map((prompt) => (
                <button
                  key={prompt}
                  type="button"
                  className="ai-starter-chip"
                  onClick={() => chat.sendMessage(prompt)}
                  disabled={querying}
                >
                  {prompt}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <div className="ai-messages-list">
            {messages.map((msg) => (
              <ChatMessageRow
                key={msg.id}
                msg={msg}
                fallbackProviderName={activeProvider.name}
                trash={trash}
              />
            ))}
          </div>
        )}

        {statusMessage && (
          <div className="ai-status-banner">
            <span className="ai-spinner" />
            <span>{statusMessage}</span>
          </div>
        )}

        {queryError && (
          <div className="ai-error-banner">
            <p>{queryError}</p>
            {!activeProvider.connected && activeProvider.installed && (
              <button type="button" className="ai-connect-btn" onClick={onOpenSettings}>
                Sign in to {activeProvider.name} in Settings
              </button>
            )}
          </div>
        )}
      </div>

      <footer className="ai-panel-footer">
        {messages.length > 0 && (
          <div className="ai-footer-toolbar">
            <button
              type="button"
              className="ai-text-btn"
              onClick={chat.clear}
              disabled={querying}
            >
              Clear conversation
            </button>
            {querying && (
              <button type="button" className="ai-text-btn danger" onClick={chat.stop}>
                Stop responding
              </button>
            )}
          </div>
        )}

        <ModelPickerRow
          choices={picker.choices}
          pick={picker.pick}
          modelChoice={picker.modelChoice}
          effortChoice={picker.effortChoice}
          catalogError={picker.catalogError}
          connected={activeProvider.connected}
          disabled={querying}
          onPick={picker.updatePick}
        />

        <form
          className="ai-input-form"
          onSubmit={(e) => {
            e.preventDefault();
            chat.sendMessage(chat.input);
          }}
        >
          <textarea
            className="ai-input-box"
            rows={2}
            placeholder={`Ask ${activeProvider.name} about your storage… (Enter to send)`}
            value={chat.input}
            onChange={(e) => chat.setInput(e.target.value)}
            onKeyDown={(e) => {
              // Enter while an input method is composing picks the candidate, not send.
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                chat.sendMessage(chat.input);
              }
            }}
            disabled={querying}
          />
          <div className="ai-input-actions">
            {querying ? (
              <button
                type="button"
                className="ai-send-btn stop"
                onClick={chat.stop}
                title="Stop generation"
              >
                Stop
              </button>
            ) : (
              <button
                type="submit"
                className="ai-send-btn"
                disabled={!chat.input.trim()}
                title="Send query"
              >
                Send
              </button>
            )}
          </div>
        </form>
      </footer>
    </aside>
  );
}
