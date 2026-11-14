import { formatBytes, isWithin } from "../shared/format";
import { CheckIcon, FileIcon, FolderIcon, RevealIcon, TrashIcon } from "../shared/icons";
import { platform } from "../shared/platform";
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
  const { fileManager, trashName, trashTarget } = platform();
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
        title={`Show item in ${fileManager}`}
      >
        <RevealIcon /> Show in {fileManager}
      </button>
      <button
        type="button"
        className="ai-action-btn trash"
        onClick={() => {
          trash.clearTrashError();
          trash.setConfirmingTrash(confirmKey);
        }}
        title={`Move this file or folder to ${trashTarget}`}
      >
        <TrashIcon /> Move to {trashName}
      </button>
    </>
  );
}

function TrashedInfo({ outcome }: { outcome: TrashOutcome | undefined }) {
  const { trashName, restoreHint } = platform();
  return (
    <div className="ai-trashed-info">
      <span className="ai-trashed-label">
        <CheckIcon /> Moved to {trashName}
      </span>
      {outcome && (
        <span className="ai-trashed-note">
          {/* Moving into the Trash keeps the bytes on disk. */}
          {outcome.freedBytes > 0 &&
            `Empty the ${trashName} to free ${formatBytes(outcome.freedBytes)}. `}
          {restoreHint(outcome.putBack)}
        </span>
      )}
    </div>
  );
}

function ConfirmTrash({ path, trash }: { path: string; trash: TrashState }) {
  const isTrashing = trash.trashingPath === path;
  const { trashName } = platform();
  return (
    <div className="ai-confirm-box">
      <span className="ai-confirm-msg">Move to {trashName}?</span>
      <button
        type="button"
        className="ai-confirm-btn danger"
        onClick={() => trash.moveToTrash(path)}
        disabled={isTrashing}
      >
        {isTrashing ? "Moving…" : `Move to ${trashName}`}
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
