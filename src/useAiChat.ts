import { useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { errorText, runningInApp } from "./format";
import type { AiProviderId, AiProviderStatus } from "./aiProviders";
import { DEFAULT_LABEL, type EffortChoice, type ModelChoice } from "./aiModels";
import type { AiQueryEvent, ChatMessage } from "./aiTypes";

type ChatOptions = {
  provider: AiProviderId;
  activeProvider: AiProviderStatus;
  currentPath: string;
  modelChoice: ModelChoice | undefined;
  effortChoice: EffortChoice | null;
};

/** The conversation with the AI CLI: messages, streaming state, send and stop. */
export function useAiChat({
  provider,
  activeProvider,
  currentPath,
  modelChoice,
  effortChoice,
}: ChatOptions) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [querying, setQuerying] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);

  const queryGen = useRef(0);
  const nextMessageId = useRef(0);
  const pendingReplyId = useRef<string | null>(null);
  const queryingRef = useRef(false);
  queryingRef.current = querying;

  // A reply still running when the panel goes away would keep the CLI working for nobody.
  useEffect(
    () => () => {
      if (queryingRef.current && runningInApp()) invoke("stop_ai_query").catch(() => {});
    },
    [],
  );

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
        provider,
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

  const stop = () => {
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

  const clear = () => {
    setMessages([]);
    setQueryError(null);
    setStatusMessage(null);
  };

  return { messages, input, setInput, querying, statusMessage, queryError, sendMessage, stop, clear };
}
