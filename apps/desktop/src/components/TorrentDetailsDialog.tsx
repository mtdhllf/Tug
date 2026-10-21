import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n";
import {
  listTorrentFiles,
  setTorrentFiles,
  torrentRuntime,
  type TaskView,
  type TorrentFileInfo,
  type TorrentRuntimeDetails,
} from "../lib/backend";
import { formatBytes, formatDuration, percentOf } from "../lib/format";
import { CheckIcon, CloseIcon, FileIcon } from "./icons";

interface TorrentDetailsDialogProps {
  task: TaskView | null;
  onClose: () => void;
}

type Tab = "files" | "peers" | "trackers";

export function TorrentDetailsDialog({ task, onClose }: TorrentDetailsDialogProps) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>("files");
  const [files, setFiles] = useState<TorrentFileInfo[]>([]);
  const [runtime, setRuntime] = useState<TorrentRuntimeDetails | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const taskId = task?.id;

  // 只在新任务打开时回到文件页；任务进度事件会不断替换 task 对象，
  // 不能以 task 作为依赖，否则 Peers/Trackers 页签会被反复重置。
  useEffect(() => {
    setTab("files");
  }, [taskId]);

  useEffect(() => {
    if (taskId === undefined) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    listTorrentFiles(taskId)
      .then((next) => {
        if (cancelled) return;
        setFiles(next);
        setSelected(new Set(next.filter((file) => file.selected).map((file) => file.index)));
      })
      .catch((cause) => {
        if (!cancelled) {
          setFiles([]);
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    torrentRuntime(taskId)
      .then((next) => {
        if (!cancelled) setRuntime(next);
      })
      .catch(() => {
        if (!cancelled) setRuntime(null);
      });
    return () => {
      cancelled = true;
    };
  }, [taskId, reloadKey]);

  const selectedBytes = useMemo(
    () => files.filter((file) => selected.has(file.index)).reduce((sum, file) => sum + file.length, 0),
    [files, selected],
  );

  if (!task) return null;
  const completed = task.state === "completed";
  const percent = percentOf(task);

  function toggle(index: number) {
    if (completed) return;
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  }

  async function save() {
    if (!task || selected.size === 0) return;
    setSaving(true);
    setError(null);
    try {
      await setTorrentFiles(task.id, [...selected].sort((a, b) => a - b));
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="modal detail-modal" role="dialog" aria-modal="true" aria-labelledby="torrent-details-title">
        <div className="detail-head">
          <div>
            <div className="modal-kicker">TASK DETAIL / BT</div>
            <h2 id="torrent-details-title">{task.name}</h2>
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t("bt.close")}><CloseIcon size={14} /></button>
        </div>

        <div className="detail-summary">
          <span className="detail-file-icon"><FileIcon size={18} /></span>
          <div className="detail-summary-text">
            <b>{task.name}</b>
            <span>{task.kind} · {task.peers} peers · {task.saveDir}</span>
          </div>
          <div className="detail-kpis">
            <div><span>PROGRESS</span><b>{percent === null ? "--" : `${percent.toFixed(1)}%`}</b></div>
            <div><span>SPEED</span><b>{formatBytes(task.speedBps)}/s</b></div>
            <div><span>ETA</span><b>{formatDuration(task.etaSecs)}</b></div>
            <div><span>RATIO</span><b>{task.downloaded > 0 ? (task.uploaded / task.downloaded).toFixed(2) : "—"}</b></div>
          </div>
        </div>

        <div className="detail-tabs">
          <button type="button" className={tab === "files" ? "active" : ""} onClick={() => setTab("files")}>{t("bt.files")} {files.length}</button>
          <button type="button" className={tab === "peers" ? "active" : ""} onClick={() => setTab("peers")}>{t("bt.peers")} {runtime?.peers.length ?? 0}</button>
          <button type="button" className={tab === "trackers" ? "active" : ""} onClick={() => setTab("trackers")}>{t("bt.trackers")} {runtime?.trackers.length ?? 0}</button>
        </div>

        <div className="detail-body">
          {loading && <div className="inline-empty">{t("bt.loading")}</div>}
          {error && <div className="form-error">{error} <button type="button" className="link-btn" onClick={() => setReloadKey((key) => key + 1)}>{t("bt.retry")}</button></div>}

          {!loading && tab === "files" && (
            <>
              <div className="detail-toolbar">
                <span>{t("bt.selected", { selected: selected.size, total: files.length })}</span>
                <span className="toolbar-spacer" />
                <button type="button" className="link-btn" onClick={() => setSelected(new Set(files.map((file) => file.index)))}>{t("bt.selectAll")}</button>
                <button type="button" className="link-btn" onClick={() => setSelected(new Set())}>{t("bt.clear")}</button>
              </div>
              <div className="file-table">
                <div className="file-row head"><span></span><span>文件</span><span>大小</span><span>进度</span><span>优先级</span><span>状态</span></div>
                {files.map((file) => (
                  <div className="file-row" key={file.index}>
                    <button type="button" className="checkbox" data-checked={selected.has(file.index)} onClick={() => toggle(file.index)} disabled={completed}>{selected.has(file.index) ? <CheckIcon size={11} /> : ""}</button>
                    <div className="cell-task">
                      <span className="file-chip http"><FileIcon size={12} /></span>
                      <div className="cell-task-text"><div className="cell-name">{file.path}</div><div className="cell-sub">#{file.index}</div></div>
                    </div>
                    <span className="cell-strong">{formatBytes(file.length)}</span>
                    <span className="cell-sub">{file.selected ? "100%" : "0%"}</span>
                    <span className="priority">{file.selected ? "普通" : "跳过"}</span>
                    <span className={`state-tag ${file.selected ? "completed" : "queued"}`}>{file.selected ? "DONE" : "WAIT"}</span>
                  </div>
                ))}
              </div>
            </>
          )}

          {!loading && tab === "peers" && (
            <div className="file-table">
              <div className="file-row peers head"><span>PEER</span><span>客户端</span><span>状态</span><span>下载</span><span>上传</span></div>
              {(runtime?.peers ?? []).length === 0 ? <div className="inline-empty">{t("bt.noPeers")}</div> : runtime?.peers.map((peer) => (
                <div className="file-row peers" key={peer.address}>
                  <span className="mono">{peer.address}</span>
                  <span className="cell-sub">{peer.client ?? "—"}</span>
                  <span className={`state-tag ${peer.state === "OK" ? "completed" : "queued"}`}>{peer.state}</span>
                  <span className="cell-strong">{formatBytes(peer.downloaded)}</span>
                  <span className="cell-strong">{formatBytes(peer.uploaded)}</span>
                </div>
              ))}
            </div>
          )}

          {!loading && tab === "trackers" && (
            <div className="tracker-list">
              {(runtime?.trackers ?? []).length === 0 ? <div className="inline-empty">{t("bt.noTrackers")}</div> : runtime?.trackers.map((tracker) => (
                <div className="tracker-row" key={tracker}>
                  <span className="status-dot" />
                  <span className="mono">{tracker}</span>
                  <span className="state-tag queued">已配置</span>
                </div>
              ))}
              <div className="form-note">Tracker 状态由引擎运行时提供；超时会在下次 announce 自动重试。</div>
            </div>
          )}
        </div>

        <div className="modal-foot">
          <span className="mono faint">{completed ? t("bt.completedLocked") : `${selected.size} / ${files.length} · ${formatBytes(selectedBytes)}`}</span>
          <span className="toolbar-spacer" />
          <button type="button" className="btn" onClick={onClose}>{t("bt.close")}</button>
          {tab === "files" && (
            <button type="button" className="btn btn-primary" onClick={save} disabled={saving || completed || loading || files.length === 0 || selected.size === 0}>
              {t("bt.save")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
