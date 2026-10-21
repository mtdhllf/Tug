import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/zh-CN";
import type { TaskState, TaskView } from "../lib/backend";
import { formatBytes, formatDuration, percentOf } from "../lib/format";
import {
  FileIcon,
  FolderIcon,
  InfoIcon,
  PauseIcon,
  PlayIcon,
  TrashIcon,
} from "./icons";

const STATE_KEYS: Record<TaskState, MessageKey> = {
  queued: "task.state.queued",
  fetching_metadata: "task.state.fetching_metadata",
  downloading: "task.state.downloading",
  paused: "task.state.paused",
  seeding: "task.state.completed",
  completed: "task.state.completed",
  failed: "task.state.failed",
};

interface TaskTableProps {
  tasks: TaskView[];
  selected: Set<number>;
  onToggle: (id: number) => void;
  onToggleAll: () => void;
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

export function TaskTable({
  tasks,
  selected,
  onToggle,
  onToggleAll,
  onPause,
  onResume,
  onRemove,
  onDetails,
  onOpen,
  onReveal,
}: TaskTableProps) {
  const { t } = useI18n();
  const allSelected = tasks.length > 0 && tasks.every((task) => selected.has(task.id));

  return (
    <div className="task-table">
      <div className="table-head">
        <button
          type="button"
          className="checkbox"
          aria-label="select all"
          data-checked={allSelected}
          onClick={onToggleAll}
        >
          {allSelected ? "✓" : ""}
        </button>
        <span>任务</span>
        <span>状态</span>
        <span>进度</span>
        <span>大小</span>
        <span>速度</span>
        <span>PEER</span>
        <span>操作</span>
      </div>
      {tasks.map((task) => {
        const percent = percentOf(task);
        const active = isActive(task);
        const resumable = task.state === "paused" || task.state === "failed";
        const completed = task.state === "completed" || task.state === "seeding";
        const checked = selected.has(task.id);
        return (
          <div className={`table-row ${checked ? "selected" : ""}`} key={task.id}>
            <button
              type="button"
              className="checkbox"
              aria-label={`select ${task.name}`}
              data-checked={checked}
              onClick={() => onToggle(task.id)}
            >
              {checked ? "✓" : ""}
            </button>
            <div className="cell-task">
              <span className={`file-chip ${task.kind === "torrent" ? "bt" : "http"}`}><FileIcon size={14} /></span>
              <div className="cell-task-text">
                <div className="cell-name">
                  <span title={task.name}>{task.name}</span>
                  <span className={`type-tag ${task.kind === "torrent" ? "bt" : "http"}`}>
                    {task.kind === "torrent" ? t("task.torrent") : t("task.http")}
                  </span>
                </div>
                <div className="cell-sub" title={task.path}>{task.error ?? task.path}</div>
              </div>
            </div>
            <div><span className={`state-tag ${task.state}`}>{t(STATE_KEYS[task.state])}</span></div>
            <div>
              <div className={`progress ${task.state === "seeding" || task.state === "completed" ? "done" : task.kind === "torrent" ? "bt" : ""}`}>
                <i style={{ width: `${percent ?? 0}%` }} />
              </div>
              <div className="cell-sub">{percent === null ? "--" : `${percent.toFixed(1)}%`}</div>
            </div>
            <div>
              <div className="cell-strong">{formatBytes(task.downloaded)} / {task.total ? formatBytes(task.total) : "?"}</div>
              <div className="cell-sub">{task.kind === "torrent" ? "BT" : "HTTP"}</div>
            </div>
            <div>
              <div className="cell-strong">
                {active ? `${formatBytes(task.speedBps)}/s` : "—"}
              </div>
              <div className="cell-sub">
                {active ? `ETA ${formatDuration(task.etaSecs)}` : ""}
              </div>
            </div>
            <div>
              <div className="cell-strong">{task.kind === "torrent" ? task.peers : "—"}</div>
              <div className="cell-sub">{task.kind === "torrent" ? "peers" : ""}</div>
            </div>
            <div className="row-actions">
              {task.kind === "torrent" && (
                <button type="button" className="icon-btn" title={t("task.details")} aria-label={t("task.details")} onClick={() => onDetails(task)}>
                  <InfoIcon size={13} />
                </button>
              )}
              {active && (
                <button type="button" className="icon-btn" title={t("task.pause")} aria-label={t("task.pause")} onClick={() => onPause(task.id)}>
                  <PauseIcon size={12} />
                </button>
              )}
              {resumable && (
                <button type="button" className="icon-btn" title={task.state === "failed" ? t("task.retry") : t("task.resume")} aria-label={task.state === "failed" ? t("task.retry") : t("task.resume")} onClick={() => onResume(task.id)}>
                  <PlayIcon size={12} />
                </button>
              )}
              {completed && (
                <>
                  <button type="button" className="icon-btn" title={t("task.open")} aria-label={t("task.open")} onClick={() => onOpen(task)}>
                    <FileIcon size={12} />
                  </button>
                  <button type="button" className="icon-btn" title={t("task.openFolder")} aria-label={t("task.openFolder")} onClick={() => onReveal(task)}>
                    <FolderIcon size={12} />
                  </button>
                </>
              )}
              <button type="button" className="icon-btn danger" title={t("task.remove")} aria-label={t("task.remove")} onClick={() => onRemove(task)}>
                <TrashIcon size={12} />
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
