import { useEffect, useRef, useState, type ReactNode } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  CheckIcon,
  CloseIcon,
  FileIcon,
  FolderIcon,
  RevealIcon,
  SparklesIcon,
  TrashIcon,
} from "./icons";
import { errorText, formatBytes, isWithin, runningInApp } from "./format";
import {
  findProvider,
  providerDotClass,
  providerStatusText,
  type AiProviderId,
  type AiProviders,
} from "./aiProviders";
import {
  DEFAULT_LABEL,
  modelChoices,
  pickEffort,
  type AiModel,
  type ModelChoice,
} from "./aiModels";

export type AiRecommendation = {
  path: string;
  name: string;
  isDir: boolean;
  bytes: number | null;
  safety: "safe" | "caution";
  reason: string;
  exists: boolean;
};

type AiQueryEvent =
  | { event: "status"; data: { message: string } }
  | { event: "chunk"; data: { text: string } }
  | {
      event: "done";
      data: { answer: string; recommendations: AiRecommendation[] };
    };

type TrashOutcome = {
  originalPath: string;
  trashPath: string;
  freedBytes: number;
  putBack: boolean;
};

type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  /** Who answered, kept so switching providers does not relabel earlier replies. */
  providerName?: string;
  modelLabel?: string;
  /** Set when the reply did not finish. Such replies are not sent back as history. */
  outcome?: "error" | "stopped";
  recommendations?: AiRecommendation[];
};

type Props = {
  ai: AiProviders;
  currentPath: string;
  hidden?: boolean;
  onClose: () => void;
  onOpenSettings: () => void;
  onItemTrashed?: (path: string, freedBytes: number) => void;
};

/** The model (by key) and effort (by label) picked for one provider. */
type ModelPick = { model: string | null; effort: string | null };

const PICK_KEY = "aiAdvisor.model.";

function readPick(provider: AiProviderId): ModelPick {
  try {
    const stored = JSON.parse(localStorage.getItem(PICK_KEY + provider) ?? "null");
    if (stored && typeof stored === "object") {
      return {
        model: typeof stored.model === "string" ? stored.model : null,
        effort: typeof stored.effort === "string" ? stored.effort : null,
      };
    }
  } catch {
    // Storage can be missing or blocked; the pickers then start at the defaults.
  }
  return { model: null, effort: null };
}

function savePick(provider: AiProviderId, pick: ModelPick) {
  try {
    localStorage.setItem(PICK_KEY + provider, JSON.stringify(pick));
  } catch {
    // Not remembering the choice is fine.
  }
}

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
  const [catalogs, setCatalogs] = useState<Partial<Record<AiProviderId, ModelChoice[]>>>({});
  const [catalogErrors, setCatalogErrors] = useState<Partial<Record<AiProviderId, string>>>({});
  const [picks, setPicks] = useState<Record<AiProviderId, ModelPick>>(() => ({
    claude: readPick("claude"),
    cursor: readPick("cursor"),
  }));

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [querying, setQuerying] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);

  // Track confirmation dialog & trashed states per path
  const [confirmingTrash, setConfirmingTrash] = useState<string | null>(null);
  const [trashingPath, setTrashingPath] = useState<string | null>(null);
  const [trashedPaths, setTrashedPaths] = useState<Map<string, TrashOutcome>>(new Map());
  const [trashError, setTrashError] = useState<{ path: string; message: string } | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const queryGen = useRef(0);
  const nextMessageId = useRef(0);
  const pendingReplyId = useRef<string | null>(null);
  const queryingRef = useRef(false);
  queryingRef.current = querying;

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

  // A reply still running when the panel goes away would keep the CLI working for nobody.
  useEffect(
    () => () => {
      if (queryingRef.current && runningInApp()) invoke("stop_ai_query").catch(() => {});
    },
    [],
  );

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, statusMessage, querying]);

  const activeProvider = findProvider(ai, selectedProvider);

  // Cursor's models depend on the account, so they are listed once signed in.
  const catalogRequested = useRef(new Set<AiProviderId>());
  useEffect(() => {
    const id = selectedProvider;
    if (!runningInApp() || !activeProvider.connected || catalogRequested.current.has(id)) return;
    catalogRequested.current.add(id);
    invoke<AiModel[]>("ai_models", { provider: id })
      .then((models) => {
        setCatalogs((prev) => ({ ...prev, [id]: modelChoices(id, models) }));
        setCatalogErrors((prev) => ({ ...prev, [id]: undefined }));
      })
      .catch((err: unknown) => {
        // Let a later visit try again, for example after signing in.
        catalogRequested.current.delete(id);
        setCatalogErrors((prev) => ({ ...prev, [id]: errorText(err) }));
      });
  }, [selectedProvider, activeProvider.connected]);

  const choices = catalogs[selectedProvider];
  const pick = picks[selectedProvider];
  const modelChoice = choices?.find((c) => c.key === pick.model) ?? choices?.[0];
  const effortChoice = modelChoice ? pickEffort(modelChoice, pick.effort) : null;

  const updatePick = (next: ModelPick) => {
    setPicks((prev) => ({ ...prev, [selectedProvider]: next }));
    savePick(selectedProvider, next);
  };
  const statusText = runningInApp()
    ? providerStatusText(ai, activeProvider)
    : "Open Storage Saver to use the AI Advisor";

  const sendMessage = async (promptText: string) => {
    const trimmed = promptText.trim();
    if (!trimmed || querying) return;
    if (!runningInApp()) {
      setQueryError("Storage Saver app is required to query AI.");
      return;
    }

    const providerName = activeProvider.name;
    const modelLabel = modelChoice
      ? [modelChoice.label, effortChoice?.label]
          .filter((label) => label && label !== DEFAULT_LABEL)
          .join(" · ")
      : "";
    const seq = ++nextMessageId.current;
    const userMsg: ChatMessage = {
      id: `u-${seq}`,
      role: "user",
      content: trimmed,
    };

    const assistantMsgId = `a-${seq}`;
    const assistantMsg: ChatMessage = {
      id: assistantMsgId,
      role: "assistant",
      content: "",
      providerName,
      modelLabel: modelLabel || undefined,
      recommendations: [],
    };

    // Failed and stopped replies are not answers, so the AI should not build on them.
    const historyPayload = messages
      .filter((m) => m.role === "user" || (!m.outcome && m.content.trim()))
      .map((m) => ({
        role: m.role,
        content: m.content,
      }));

    setMessages((prev) => [...prev, userMsg, assistantMsg]);
    setInput("");
    setQuerying(true);
    setQueryError(null);
    setStatusMessage("Gathering storage data…");

    const gen = ++queryGen.current;
    pendingReplyId.current = assistantMsgId;

    const channel = new Channel<AiQueryEvent>();
    channel.onmessage = (event) => {
      if (queryGen.current !== gen) return;
      if (event.event === "status") {
        setStatusMessage(event.data.message);
      } else if (event.event === "chunk") {
        setStatusMessage(null);
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMsgId ? { ...m, content: event.data.text } : m,
          ),
        );
      } else if (event.event === "done") {
        setStatusMessage(null);
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMsgId
              ? {
                  ...m,
                  content: event.data.answer,
                  recommendations: event.data.recommendations,
                }
              : m,
          ),
        );
        setQuerying(false);
      }
    };

    try {
      await invoke("query_ai", {
        provider: selectedProvider,
        prompt: trimmed,
        currentPath,
        history: historyPayload,
        model: effortChoice?.model ?? null,
        effort: effortChoice?.effort ?? null,
        onEvent: channel,
      });
      if (queryGen.current === gen) {
        setQuerying(false);
        setStatusMessage(null);
      }
    } catch (err: unknown) {
      if (queryGen.current === gen) {
        const errStr = errorText(err);
        setQueryError(errStr);
        setQuerying(false);
        setStatusMessage(null);
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantMsgId
              ? {
                  ...m,
                  content: m.content || `Could not query ${providerName}: ${errStr}`,
                  outcome: "error",
                }
              : m,
          ),
        );
      }
    }
  };

  const handleStop = () => {
    queryGen.current += 1;
    setQuerying(false);
    setStatusMessage(null);
    const stoppedId = pendingReplyId.current;
    setMessages((prev) =>
      prev.map((m) => (m.id === stoppedId ? { ...m, outcome: "stopped" } : m)),
    );
    if (runningInApp()) {
      invoke("stop_ai_query").catch(() => {});
    }
  };

  const handleReveal = (path: string) => {
    if (!runningInApp()) return;
    revealItemInDir(path).catch((err: unknown) => {
      setTrashError({ path, message: errorText(err) });
    });
  };

  const handleMoveToTrash = async (path: string) => {
    setTrashingPath(path);
    setTrashError(null);
    try {
      const result = await invoke<TrashOutcome>("move_to_trash", { path });
      setTrashedPaths((prev) => new Map(prev).set(path, result));
      setConfirmingTrash(null);
      if (onItemTrashed) {
        onItemTrashed(path, result.freedBytes);
      }
    } catch (err: unknown) {
      setTrashError({ path, message: errorText(err) });
    } finally {
      setTrashingPath(null);
    }
  };

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
                  onClick={() => sendMessage(prompt)}
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
              <div key={msg.id} className={`ai-message-row ${msg.role}`}>
                <div className="ai-message-bubble">
                  {msg.role === "assistant" && (
                    <div className="ai-bubble-badge">
                      <SparklesIcon />
                      <span>
                        {msg.providerName ?? activeProvider.name}
                        {msg.modelLabel && ` · ${msg.modelLabel}`}
                      </span>
                    </div>
                  )}
                  <div className="ai-message-text">{renderMarkdown(msg.content)}</div>
                  {msg.outcome === "stopped" && (
                    <p className="ai-message-note">
                      {msg.content ? "Stopped." : "Stopped before answering."}
                    </p>
                  )}

                  {msg.recommendations && msg.recommendations.length > 0 && (
                    <div className="ai-recommendations-section">
                      <h4 className="ai-recs-title">
                        Recommended for cleanup ({msg.recommendations.length})
                      </h4>
                      <div className="ai-recs-list">
                        {msg.recommendations.map((rec) => {
                          // Trashing a folder takes everything inside it along.
                          const isTrashed = [...trashedPaths.keys()].some((trashed) =>
                            isWithin(rec.path, trashed),
                          );
                          const trashOutcome = trashedPaths.get(rec.path);
                          const confirmKey = `${msg.id}\n${rec.path}`;
                          const isConfirming = confirmingTrash === confirmKey;
                          const isTrashing = trashingPath === rec.path;
                          const hasError =
                            trashError && trashError.path === rec.path
                              ? trashError.message
                              : null;

                          return (
                            <div
                              key={rec.path}
                              className={`ai-rec-card ${isTrashed ? "trashed" : ""} ${
                                rec.safety
                              }`}
                            >
                              <div className="ai-rec-header">
                                <span className="ai-rec-icon">
                                  {rec.isDir ? <FolderIcon /> : <FileIcon />}
                                </span>
                                <div className="ai-rec-info">
                                  <span className="ai-rec-name" title={rec.name}>
                                    {rec.name}
                                  </span>
                                  <span className="ai-rec-path" title={rec.path}>
                                    {rec.path}
                                  </span>
                                </div>
                                <div className="ai-rec-meta">
                                  {rec.bytes !== null && (
                                    <span className="ai-rec-size">
                                      {formatBytes(rec.bytes)}
                                    </span>
                                  )}
                                  <span className={`ai-safety-pill ${rec.safety}`}>
                                    {rec.safety === "safe" ? "Safe" : "Review"}
                                  </span>
                                </div>
                              </div>

                              {rec.reason && (
                                <p className="ai-rec-reason">{rec.reason}</p>
                              )}

                              {hasError && (
                                <p className="ai-rec-error">{hasError}</p>
                              )}

                              <div className="ai-rec-actions">
                                {isTrashed ? (
                                  <div className="ai-trashed-info">
                                    <span className="ai-trashed-label">
                                      <CheckIcon /> Moved to Trash
                                    </span>
                                    {trashOutcome && (
                                      <span className="ai-trashed-note">
                                        {/* Moving into the Trash keeps the bytes on disk. */}
                                        {trashOutcome.freedBytes > 0 &&
                                          `Empty the Trash to free ${formatBytes(trashOutcome.freedBytes)}. `}
                                        {trashOutcome.putBack
                                          ? "Restore it with Put Back in Finder."
                                          : "Put Back is unavailable: allow Storage Saver to control Finder in System Settings › Privacy & Security › Automation."}
                                      </span>
                                    )}
                                  </div>
                                ) : !rec.exists ? (
                                  <span className="ai-rec-missing">Not found on disk</span>
                                ) : isConfirming ? (
                                  <div className="ai-confirm-box">
                                    <span className="ai-confirm-msg">
                                      Move to Trash?
                                    </span>
                                    <button
                                      type="button"
                                      className="ai-confirm-btn danger"
                                      onClick={() => handleMoveToTrash(rec.path)}
                                      disabled={isTrashing}
                                    >
                                      {isTrashing ? "Moving…" : "Move to Trash"}
                                    </button>
                                    <button
                                      type="button"
                                      className="ai-confirm-btn cancel"
                                      onClick={() => setConfirmingTrash(null)}
                                      disabled={isTrashing}
                                    >
                                      Cancel
                                    </button>
                                  </div>
                                ) : (
                                  <>
                                    <button
                                      type="button"
                                      className="ai-action-btn reveal"
                                      onClick={() => handleReveal(rec.path)}
                                      title="Show item in Finder"
                                    >
                                      <RevealIcon /> Show in Finder
                                    </button>
                                    <button
                                      type="button"
                                      className="ai-action-btn trash"
                                      onClick={() => {
                                        setTrashError(null);
                                        setConfirmingTrash(confirmKey);
                                      }}
                                      title="Move this file or folder to macOS Trash"
                                    >
                                      <TrashIcon /> Move to Trash
                                    </button>
                                  </>
                                )}
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )}
                </div>
              </div>
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
              onClick={() => {
                setMessages([]);
                setQueryError(null);
                setStatusMessage(null);
              }}
              disabled={querying}
            >
              Clear conversation
            </button>
            {querying && (
              <button
                type="button"
                className="ai-text-btn danger"
                onClick={handleStop}
              >
                Stop responding
              </button>
            )}
          </div>
        )}

        <div className="ai-model-row">
          <label className="ai-model-field">
            <span>Model</span>
            <select
              value={modelChoice?.key ?? ""}
              onChange={(e) => updatePick({ model: e.target.value, effort: pick.effort })}
              disabled={querying || !choices}
              title={catalogErrors[selectedProvider]}
            >
              {choices ? (
                choices.map((choice) => (
                  <option key={choice.key} value={choice.key}>
                    {choice.label}
                  </option>
                ))
              ) : (
                <option value="">
                  {catalogErrors[selectedProvider]
                    ? "Could not list models"
                    : activeProvider.connected
                      ? "Loading…"
                      : "Default"}
                </option>
              )}
            </select>
          </label>
          <label className="ai-model-field">
            <span>Effort</span>
            <select
              value={effortChoice?.label ?? ""}
              onChange={(e) => updatePick({ model: pick.model, effort: e.target.value })}
              disabled={querying || !modelChoice || modelChoice.efforts.length < 2}
            >
              {modelChoice ? (
                modelChoice.efforts.map((effort) => (
                  <option key={effort.label} value={effort.label}>
                    {effort.label}
                  </option>
                ))
              ) : (
                <option value="">Default</option>
              )}
            </select>
          </label>
        </div>

        <form
          className="ai-input-form"
          onSubmit={(e) => {
            e.preventDefault();
            sendMessage(input);
          }}
        >
          <textarea
            className="ai-input-box"
            rows={2}
            placeholder={`Ask ${activeProvider.name} about your storage… (Enter to send)`}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              // Enter while an input method is composing picks the candidate, not send.
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                sendMessage(input);
              }
            }}
            disabled={querying}
          />
          <div className="ai-input-actions">
            {querying ? (
              <button
                type="button"
                className="ai-send-btn stop"
                onClick={handleStop}
                title="Stop generation"
              >
                Stop
              </button>
            ) : (
              <button
                type="submit"
                className="ai-send-btn"
                disabled={!input.trim()}
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

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d+)[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const TABLE_DIVIDER = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/** Read the markdown the AI writes line by line: lists need not follow a blank line. */
function renderMarkdown(text: string) {
  if (!text) return null;
  const lines = text.split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  const startsBlock = (line: string) =>
    line.trim().startsWith("```") ||
    HEADING.test(line.trim()) ||
    BULLET.test(line) ||
    NUMBERED.test(line) ||
    line.trim().startsWith("|");

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();
    const key = blocks.length;
    if (!trimmed) {
      i += 1;
      continue;
    }
    if (trimmed.startsWith("```")) {
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        code.push(lines[i]);
        i += 1;
      }
      i += 1;
      blocks.push(
        <pre key={key}>
          <code>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    const heading = HEADING.exec(trimmed);
    if (heading) {
      const content = formatInline(heading[2]);
      const level = heading[1].length;
      blocks.push(
        level === 1 ? (
          <h2 key={key}>{content}</h2>
        ) : level === 2 ? (
          <h3 key={key}>{content}</h3>
        ) : (
          <h4 key={key}>{content}</h4>
        ),
      );
      i += 1;
      continue;
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) {
      blocks.push(<hr key={key} />);
      i += 1;
      continue;
    }
    if (BULLET.test(line)) {
      const items: string[] = [];
      while (i < lines.length && BULLET.test(lines[i])) {
        items.push(BULLET.exec(lines[i])![1]);
        i += 1;
      }
      blocks.push(
        <ul key={key}>
          {items.map((item, itemIdx) => (
            <li key={itemIdx}>{formatInline(item)}</li>
          ))}
        </ul>,
      );
      continue;
    }
    if (NUMBERED.test(line)) {
      // Items split by blank lines or sub-bullets keep their own numbers.
      const start = Number(NUMBERED.exec(line)![1]);
      const items: string[] = [];
      while (i < lines.length && NUMBERED.test(lines[i])) {
        items.push(NUMBERED.exec(lines[i])![2]);
        i += 1;
      }
      blocks.push(
        <ol key={key} start={start}>
          {items.map((item, itemIdx) => (
            <li key={itemIdx}>{formatInline(item)}</li>
          ))}
        </ol>,
      );
      continue;
    }
    if (trimmed.startsWith("|") && i + 1 < lines.length && TABLE_DIVIDER.test(lines[i + 1].trim())) {
      const cells = (row: string) =>
        row
          .trim()
          .replace(/^\|/, "")
          .replace(/\|$/, "")
          .split("|")
          .map((cell) => cell.trim());
      const header = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        rows.push(cells(lines[i]));
        i += 1;
      }
      blocks.push(
        <div key={key} className="ai-table-wrap">
          <table>
            <thead>
              <tr>
                {header.map((cell, cellIdx) => (
                  <th key={cellIdx}>{formatInline(cell)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIdx) => (
                <tr key={rowIdx}>
                  {row.map((cell, cellIdx) => (
                    <td key={cellIdx}>{formatInline(cell)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    const paragraph: string[] = [trimmed];
    i += 1;
    while (i < lines.length && lines[i].trim() && !startsBlock(lines[i])) {
      paragraph.push(lines[i].trim());
      i += 1;
    }
    blocks.push(
      <p key={key}>
        {paragraph.map((part, partIdx) => (
          <span key={partIdx}>
            {partIdx > 0 && <br />}
            {formatInline(part)}
          </span>
        ))}
      </p>,
    );
  }
  return <>{blocks}</>;
}

function formatInline(text: string) {
  const parts = text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g);
  return parts.map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`")) {
      return <code key={index}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={index}>{part.slice(2, -2)}</strong>;
    }
    return part;
  });
}
