import { formatBytes, isWithin } from "../shared/format";
import { CheckIcon, FileIcon, FolderIcon, RevealIcon, TrashIcon } from "../shared/icons";
import type { AiRecommendation, TrashOutcome } from "./aiTypes";
import type { TrashState } from "./useTrash";

type Props = {
  rec: AiRecommendation;
  /** Scopes the confirm prompt to this reply when the same path is recommended twice. */
  messageId: string;
  trash: TrashState;
};

export function RecommendationCard({ rec, messageId, trash }: Props) {
  const hasError = trash.trashError?.path === rec.path ? trash.trashError.message : null;

  return (
    <div className={`ai-rec-card ${isTrashed(rec, trash) ? "trashed" : ""} ${rec.safety}`}>
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
        <RecActions rec={rec} confirmKey={`${messageId}\n${rec.path}`} trash={trash} />
      </div>
    </div>
  );
}

/** Trashing a folder takes everything inside it along. */
function isTrashed(rec: AiRecommendation, trash: TrashState) {
  return [...trash.trashedPaths.keys()].some((trashed) => isWithin(rec.path, trashed));
}

type ActionsProps = {
  rec: AiRecommendation;
  /** Scopes the confirm prompt to one reply's card. */
  confirmKey: string;
  trash: TrashState;
};

function RecActions({ rec, confirmKey, trash }: ActionsProps) {
  if (isTrashed(rec, trash)) return <TrashedInfo outcome={trash.trashedPaths.get(rec.path)} />;
  if (!rec.exists) return <span className="ai-rec-missing">Not found on disk</span>;
  if (trash.confirmingTrash === confirmKey) {
    return <ConfirmTrash path={rec.path} trash={trash} />;
  }
  return (
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
  );
}

function TrashedInfo({ outcome }: { outcome: TrashOutcome | undefined }) {
  return (
    <div className="ai-trashed-info">
      <span className="ai-trashed-label">
        <CheckIcon /> Moved to Trash
      </span>
      {outcome && (
        <span className="ai-trashed-note">
          {/* Moving into the Trash keeps the bytes on disk. */}
          {outcome.freedBytes > 0 && `Empty the Trash to free ${formatBytes(outcome.freedBytes)}. `}
          {outcome.putBack
            ? "Restore it with Put Back in Finder."
            : "Put Back is unavailable: allow Storage Saver to control Finder in System Settings › Privacy & Security › Automation."}
        </span>
      )}
    </div>
  );
}

function ConfirmTrash({ path, trash }: { path: string; trash: TrashState }) {
  const isTrashing = trash.trashingPath === path;
  return (
    <div className="ai-confirm-box">
      <span className="ai-confirm-msg">Move to Trash?</span>
      <button
        type="button"
        className="ai-confirm-btn danger"
        onClick={() => trash.moveToTrash(path)}
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
  );
}
