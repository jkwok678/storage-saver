import { CheckIcon, FileIcon, FolderIcon, RevealIcon, TrashIcon } from "./icons";
import { formatBytes, isWithin } from "./format";
import type { AiRecommendation } from "./aiTypes";
import type { TrashState } from "./useTrash";

type Props = {
  rec: AiRecommendation;
  /** Scopes the confirm prompt to this reply when the same path is recommended twice. */
  messageId: string;
  trash: TrashState;
};

export function RecommendationCard({ rec, messageId, trash }: Props) {
  // Trashing a folder takes everything inside it along.
  const isTrashed = [...trash.trashedPaths.keys()].some((trashed) =>
    isWithin(rec.path, trashed),
  );
  const trashOutcome = trash.trashedPaths.get(rec.path);
  const confirmKey = `${messageId}\n${rec.path}`;
  const isConfirming = trash.confirmingTrash === confirmKey;
  const isTrashing = trash.trashingPath === rec.path;
  const hasError =
    trash.trashError && trash.trashError.path === rec.path ? trash.trashError.message : null;

  return (
    <div className={`ai-rec-card ${isTrashed ? "trashed" : ""} ${rec.safety}`}>
      <div className="ai-rec-header">
        <span className="ai-rec-icon">{rec.isDir ? <FolderIcon /> : <FileIcon />}</span>
        <div className="ai-rec-info">
          <span className="ai-rec-name" title={rec.name}>
            {rec.name}
          </span>
          <span className="ai-rec-path" title={rec.path}>
            {rec.path}
          </span>
        </div>
        <div className="ai-rec-meta">
          {rec.bytes !== null && <span className="ai-rec-size">{formatBytes(rec.bytes)}</span>}
          <span className={`ai-safety-pill ${rec.safety}`}>
            {rec.safety === "safe" ? "Safe" : "Review"}
          </span>
        </div>
      </div>

      {rec.reason && <p className="ai-rec-reason">{rec.reason}</p>}

      {hasError && <p className="ai-rec-error">{hasError}</p>}

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
            <span className="ai-confirm-msg">Move to Trash?</span>
            <button
              type="button"
              className="ai-confirm-btn danger"
              onClick={() => trash.moveToTrash(rec.path)}
              disabled={isTrashing}
            >
              {isTrashing ? "Moving…" : "Move to Trash"}
            </button>
            <button
              type="button"
              className="ai-confirm-btn cancel"
              onClick={() => trash.setConfirmingTrash(null)}
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
              onClick={() => trash.reveal(rec.path)}
              title="Show item in Finder"
            >
              <RevealIcon /> Show in Finder
            </button>
            <button
              type="button"
              className="ai-action-btn trash"
              onClick={() => {
                trash.clearTrashError();
                trash.setConfirmingTrash(confirmKey);
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
}
