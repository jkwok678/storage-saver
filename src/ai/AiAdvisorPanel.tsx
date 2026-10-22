import { useEffect, useRef, useState } from "react";
import { runningInApp } from "../shared/format";
import { CloseIcon, SparklesIcon } from "../shared/icons";
import {
  type AiProviderId,
  type AiProviderStatus,
  type AiProviders,
  findProvider,
  PROVIDER_IDS,
  providerDotClass,
  providerName,
  providerStatusText,
} from "./aiProviders";
import { ChatMessageRow } from "./ChatMessageRow";
import { ModelPickerRow } from "./ModelPickerRow";
import { type AiChat, useAiChat } from "./useAiChat";
import { useModelPicker } from "./useModelPicker";
import { type TrashState, useTrash } from "./useTrash";
import "./aiAdvisor.css";

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
  const [selectedProvider, setSelectedProvider] = useInitialProvider(ai.providers);
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

      <ProviderSection
        ai={ai}
        selected={selectedProvider}
        provider={activeProvider}
        onSelect={setSelectedProvider}
        onOpenSettings={onOpenSettings}
      />

      <ChatBody
        chat={chat}
        provider={activeProvider}
        trash={trash}
        onOpenSettings={onOpenSettings}
      />

      <footer className="ai-panel-footer">
        {chat.messages.length > 0 && (
          <div className="ai-footer-toolbar">
            <button
              type="button"
              className="ai-text-btn"
              onClick={chat.clear}
              disabled={chat.querying}
            >
              Clear conversation
            </button>
            {chat.querying && (
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
          disabled={chat.querying}
          onPick={picker.updatePick}
        />

        <Composer chat={chat} providerName={activeProvider.name} />
      </footer>
    </aside>
  );
}

/** Starts on a signed-in provider, once: a later refresh must not override the user's choice. */
function useInitialProvider(statuses: AiProviderStatus[] | null) {
  const [selected, setSelected] = useState<AiProviderId>("claude");
  const picked = useRef(false);
  useEffect(() => {
    if (!statuses || picked.current) return;
    picked.current = true;
    setSelected((current) => {
      if (statuses.some((s) => s.id === current && s.connected)) return current;
      return statuses.find((s) => s.connected)?.id ?? current;
    });
  }, [statuses]);
  return [selected, setSelected] as const;
}

type ProviderSectionProps = {
  ai: AiProviders;
  selected: AiProviderId;
  provider: AiProviderStatus;
  onSelect: (id: AiProviderId) => void;
  onOpenSettings: () => void;
};

function ProviderSection({
  ai,
  selected,
  provider,
  onSelect,
  onOpenSettings,
}: ProviderSectionProps) {
  const statusText = runningInApp()
    ? providerStatusText(ai, provider)
    : "Open Storage Saver to use the AI Advisor";
  const canConnect = !ai.checking && !provider.connected && provider.installed;

  return (
    <section className="ai-auth-section">
      <div className="ai-provider-toggle" role="tablist" aria-label="AI Provider">
        {PROVIDER_IDS.map((id) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={selected === id}
            className={selected === id ? "active" : ""}
            onClick={() => onSelect(id)}
          >
            {providerName(id)}
          </button>
        ))}
      </div>

      <div className="ai-auth-status">
        <div className="ai-status-indicator">
          <span className={`status-dot ${providerDotClass(ai, provider)}`} />
          <span className="status-text">{statusText}</span>
        </div>

        <div className="ai-auth-actions">
          <button
            type="button"
            className="ai-text-btn"
            onClick={onOpenSettings}
            title="Connect or sign out of accounts in Settings"
          >
            {canConnect ? "Connect in Settings" : "Manage accounts"}
          </button>
        </div>
      </div>
    </section>
  );
}

type ChatBodyProps = {
  chat: AiChat;
  provider: AiProviderStatus;
  trash: TrashState;
  onOpenSettings: () => void;
};

function ChatBody({ chat, provider, trash, onOpenSettings }: ChatBodyProps) {
  const { messages, querying, statusMessage, queryError } = chat;
  const scrollRef = useRef<HTMLDivElement>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: each new message or status scrolls to the end.
  useEffect(() => {
    const scroller = scrollRef.current;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }, [messages, statusMessage, querying]);

  return (
    <div className="ai-chat-scroll" ref={scrollRef}>
      {messages.length === 0 ? (
        <EmptyChat providerName={provider.name} disabled={querying} onAsk={chat.sendMessage} />
      ) : (
        <div className="ai-messages-list">
          {messages.map((msg) => (
            <ChatMessageRow
              key={msg.id}
              msg={msg}
              fallbackProviderName={provider.name}
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
          {!provider.connected && provider.installed && (
            <button type="button" className="ai-connect-btn" onClick={onOpenSettings}>
              Sign in to {provider.name} in Settings
            </button>
          )}
        </div>
      )}
    </div>
  );
}

type EmptyChatProps = {
  providerName: string;
  disabled: boolean;
  onAsk: (prompt: string) => void;
};

function EmptyChat({ providerName, disabled, onAsk }: EmptyChatProps) {
  return (
    <div className="ai-empty-state">
      <div className="ai-empty-icon">
        <SparklesIcon />
      </div>
      <h3>Ask anything about your disk</h3>
      <p>
        Uses your {providerName} subscription. The AI queries your scanned folder tree, largest
        files, and inspects disk paths in read-only mode to find what can be safely deleted.
      </p>
      <div className="ai-starter-list">
        <span className="ai-starter-title">Suggested questions:</span>
        {STARTER_PROMPTS.map((prompt) => (
          <button
            key={prompt}
            type="button"
            className="ai-starter-chip"
            onClick={() => onAsk(prompt)}
            disabled={disabled}
          >
            {prompt}
          </button>
        ))}
      </div>
    </div>
  );
}

function Composer({ chat, providerName }: { chat: AiChat; providerName: string }) {
  return (
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
        placeholder={`Ask ${providerName} about your storage… (Enter to send)`}
        value={chat.input}
        onChange={(e) => chat.setInput(e.target.value)}
        onKeyDown={(e) => {
          // Enter while an input method is composing picks the candidate, not send.
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            chat.sendMessage(chat.input);
          }
        }}
        disabled={chat.querying}
      />
      <div className="ai-input-actions">
        {chat.querying ? (
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
  );
}
