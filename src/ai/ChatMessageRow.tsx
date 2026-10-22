import { SparklesIcon } from "../shared/icons";
import type { ChatMessage } from "./aiTypes";
import { renderMarkdown } from "./markdown";
import { RecommendationCard } from "./RecommendationCard";
import type { TrashState } from "./useTrash";

type Props = {
  msg: ChatMessage;
  /** Label for a reply that did not record who answered it. */
  fallbackProviderName: string;
  trash: TrashState;
};

export function ChatMessageRow({ msg, fallbackProviderName, trash }: Props) {
  return (
    <div className={`ai-message-row ${msg.role}`}>
      <div className="ai-message-bubble">
        {msg.role === "assistant" && (
          <div className="ai-bubble-badge">
            <SparklesIcon />
            <span>
              {msg.providerName ?? fallbackProviderName}
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
              {msg.recommendations.map((rec) => (
                <RecommendationCard key={rec.path} rec={rec} messageId={msg.id} trash={trash} />
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
