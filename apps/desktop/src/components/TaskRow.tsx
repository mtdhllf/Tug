import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/zh-CN";
import type { TaskView } from "../lib/backend";
import { formatBytes, formatDuration, percentOf } from "../lib/format";
import {
  FileIcon,
  FolderIcon,
  InfoIcon,
  PauseIcon,
  PlayIcon,
  TrashIcon,
} from "./icons";

interface TaskRowProps {
  task: TaskView;
  onPause: (id: number) => void;
  onResume: (id: number) => void;
  onRemove: (task: TaskView) => void;
  onDetails: (task: TaskView) => void;
  onOpen: (task: TaskView) => void;
  onReveal: (task: TaskView) => void;
}

function isActive(task: TaskView) {
  return task.state === "downloading" || task.state === "fetching_metadata";
}

function stateLabel(task: TaskView, t: (key: MessageKey) => string) {
  const key = task.state === "seeding" ? "task.state.completed" : `task.state.${task.state}`;
  return t(key as MessageKey);
}

export function TaskRow({
  task,
  onPause,
  onResume,
  onRemove,
  onDetails,
  onOpen,
  onReveal,
}: TaskRowProps) {
  const { t } = useI18n();
  const percent = percentOf(task);
  const active = isActive(task);
  const resumable = task.state === "paused" || task.state === "failed";
  const completed = task.state === "completed" || task.state === "seeding";
  const isTorrent = task.kind === "torrent";

  return (
    <article className={`task-row state-${task.state}`}>
      <div className={`task-icon ${isTorrent ? "bt" : "http"} ${completed ? "seed" : ""}`}>
        <FileIcon size={18} />
      </div>

      <div className="task-main">
        <div className="task-title">
          <span className="task-name" title={task.name}>{task.name}</span>
          <span className={`type-tag ${isTorrent ? "bt" : "http"}`}>{isTorrent ? t("task.torrent") : t("task.http")}</span>
          <span className={`state-tag ${task.state}`}>{stateLabel(task, t)}</span>
        </div>
        <div className="task-path" title={task.path}>{task.path}</div>
        <div className="task-progress-row">
          <div className={`progress ${completed ? "done" : isTorrent ? "bt" : ""}`}>
            <i style={{ width: `${percent ?? 0}%` }} />
          </div>
          <span className="task-percent">{percent === null ? "--" : `${percent.toFixed(1)}%`}</span>
        </div>
        <div className="task-meta">
          <span>{formatBytes(task.downloaded)} / {task.total ? formatBytes(task.total) : "?"}</span>
          {isTorrent && <span>{t("overview.peers", { peers: task.peers })}</span>}
          {task.error && <span className="task-error" title={task.error}>{task.error}</span>}
        </div>
      </div>

      <div className="task-side">
        {active ? (
          <>
            <strong>{formatBytes(task.speedBps)}/s</strong>
            <span>{t("task.etaShort", { eta: formatDuration(task.etaSecs) })}</span>
          </>
        ) : task.state === "queued" ? (
          <>
            <strong>—</strong>
            <span>{t("task.queued")}</span>
          </>
        ) : completed ? (
          <>
            <strong>{formatBytes(task.total ?? task.downloaded)}</strong>
            <span>{t("task.completed")}</span>
          </>
        ) : (
          <>
            <strong>—</strong>
            <span>{stateLabel(task, t)}</span>
          </>
        )}
      </div>

      <div className="task-actions">
        {isTorrent && (
          <button type="button" className="icon-btn" title={t("task.details")} aria-label={t("task.details")} onClick={() => onDetails(task)}>
            <InfoIcon size={14} />
          </button>
        )}
        {active && (
          <button type="button" className="icon-btn" title={t("task.pause")} aria-label={t("task.pause")} onClick={() => onPause(task.id)}>
            <PauseIcon size={13} />
          </button>
        )}
        {resumable && (
          <button type="button" className="icon-btn" title={task.state === "failed" ? t("task.retry") : t("task.resume")} aria-label={task.state === "failed" ? t("task.retry") : t("task.resume")} onClick={() => onResume(task.id)}>
            <PlayIcon size={13} />
          </button>
        )}
        {completed && (
          <>
            <button type="button" className="icon-btn" title={t("task.open")} aria-label={t("task.open")} onClick={() => onOpen(task)}>
              <FileIcon size={13} />
            </button>
            <button type="button" className="icon-btn" title={t("task.openFolder")} aria-label={t("task.openFolder")} onClick={() => onReveal(task)}>
              <FolderIcon size={13} />
            </button>
          </>
        )}
        <button type="button" className="icon-btn danger" title={t("task.remove")} aria-label={t("task.remove")} onClick={() => onRemove(task)}>
          <TrashIcon size={13} />
        </button>
      </div>
    </article>
  );
}
