import { useMemo, useState, type FormEvent } from "react";
import { useI18n } from "../i18n";
import type { AppInfo, Settings, TaskView } from "../lib/backend";
import { formatBytes } from "../lib/format";
import type { NavCounts, View } from "./Sidebar";
import { TaskRow } from "./TaskRow";
import {
  ArrowRightIcon,
  DownloadIcon,
  HardDriveIcon,
  WifiIcon,
} from "./icons";

interface OverviewProps {
  tasks: TaskView[];
  counts: NavCounts;
  settings: Settings;
  info: AppInfo | null;
  onQuickAdd: (url: string) => void;
  onNew: () => void;
  onNavigate: (view: View) => void;
  onPause: (id: number) => void;
  onResume: (id: number) => void;
  onRemove: (task: TaskView) => void;
  onDetails: (task: TaskView) => void;
  onOpen: (task: TaskView) => void;
  onReveal: (task: TaskView) => void;
}

function speed(task: TaskView) {
  return task.speedBps + task.uploadSpeedBps;
}

export function Overview({
  tasks,
  counts,
  settings,
  info,
  onQuickAdd,
  onNew,
  onNavigate,
  onPause,
  onResume,
  onRemove,
  onDetails,
  onOpen,
  onReveal,
}: OverviewProps) {
  const { t } = useI18n();
  const [url, setUrl] = useState("");

  const active = useMemo(
    () => tasks.filter((task) => task.state !== "completed" && task.state !== "seeding"),
    [tasks],
  );
  const recent = useMemo(
    () => tasks.filter((task) => task.state === "completed" || task.state === "seeding").slice(0, 5),
    [tasks],
  );
  const totalSpeed = active.reduce((sum, task) => sum + speed(task), 0);
  const uploadSpeed = tasks.reduce((sum, task) => sum + task.uploadSpeedBps, 0);

  function submit(event: FormEvent) {
    event.preventDefault();
    const value = url.trim();
    if (!value) return;
    onQuickAdd(value);
    setUrl("");
  }

  return (
    <div className="overview">
      <section className="card hero">
        <div className="hero-kicker"><span /> CONTROL ROOM / {t("nav.overview")}</div>
        <h1>{t("overview.heroLead")}<br /><span>{t("overview.heroAccent")}</span></h1>
        <p>{t("overview.heroDesc")}</p>
        <form className="quick-add" onSubmit={submit}>
          <span className="quick-prefix">URL</span>
          <input
            value={url}
            onChange={(event) => setUrl(event.currentTarget.value)}
            placeholder={t("overview.quickPlaceholder")}
            spellCheck={false}
          />
          <button type="submit">{t("overview.quickStart")}</button>
        </form>
        <div className="hero-stats">
          <div><span>{t("overview.stat.active")}</span><strong>{String(counts.active).padStart(2, "0")} <small>{t("overview.unit.slots", { slots: settings.maxConcurrentTasks })}</small></strong></div>
          <div><span>{t("overview.stat.done")}</span><strong>{String(counts.completed).padStart(2, "0")} <small>{t("overview.unit.tasks")}</small></strong></div>
          <div><span>下载目录</span><strong className="path-value" title={info?.downloadDir ?? settings.downloadDir}>{settings.downloadDir}</strong></div>
        </div>
      </section>

      <section className="card telemetry">
        <div className="card-head"><h2>TELEMETRY</h2><span className="kicker">/ 实时吞吐</span><span className="spacer" /><span className={`live-dot ${totalSpeed > 0 ? "" : "idle"}`} /> {totalSpeed > 0 ? "STREAMING" : "IDLE"}</div>
        <div className="telemetry-main">
          <div className="telemetry-speed">{formatBytes(totalSpeed)}<small>/s</small></div>
          <div className="telemetry-chart">
            <svg viewBox="0 0 300 88" preserveAspectRatio="none" aria-hidden="true">
              <defs>
                <linearGradient id="mini-fill" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0" stopColor="var(--accent)" stopOpacity=".24" />
                  <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
                </linearGradient>
              </defs>
              {totalSpeed > 0 ? (
                <>
                  <path d="M0,70 L16,64 L30,68 L46,42 L62,54 L78,34 L96,42 L112,22 L128,34 L148,26 L166,44 L184,18 L202,32 L220,14 L238,26 L256,18 L274,34 L292,22 L300,28 L300,88 L0,88Z" fill="url(#mini-fill)" />
                  <path d="M0,70 L16,64 L30,68 L46,42 L62,54 L78,34 L96,42 L112,22 L128,34 L148,26 L166,44 L184,18 L202,32 L220,14 L238,26 L256,18 L274,34 L292,22 L300,28" fill="none" stroke="var(--accent)" strokeWidth="2" />
                </>
              ) : (
                <path d="M0,78 L300,78" fill="none" stroke="var(--line)" strokeWidth="1.5" strokeDasharray="4 4" />
              )}
            </svg>
          </div>
        </div>
        <div className="telemetry-foot">
          <span>ACTIVE <b>{counts.active}</b></span>
          <span>PEERS <b>{active.reduce((sum, task) => sum + task.peers, 0)}</b></span>
          <span>COMPLETED <b>{counts.completed}</b></span>
        </div>
      </section>

      <section className="card active-list">
        <div className="card-head">
          <h2>{t("overview.active")}</h2>
          <span className="kicker">/ {String(active.length).padStart(2, "0")} ACTIVE</span>
          <span className="spacer" />
          <button type="button" className="chip" onClick={() => active.filter((task) => ["downloading", "fetching_metadata"].includes(task.state)).forEach((task) => onPause(task.id))}>{t("overview.pauseAll")}</button>
          <button type="button" className="chip" onClick={() => onNavigate("active")}>{t("overview.viewAll")} <ArrowRightIcon size={11} /></button>
        </div>
        {active.length === 0 ? (
          <div className="overview-empty">
            <span className="overview-empty-icon"><DownloadIcon size={20} /></span>
            <b>{t("overview.emptyTitle")}</b>
            <span>{t("overview.emptyHint")}</span>
            <button type="button" className="btn btn-primary btn-sm" onClick={onNew}>{t("toolbar.new")}</button>
          </div>
        ) : (
          active.slice(0, 3).map((task) => (
            <TaskRow
              key={task.id}
              task={task}
              onPause={onPause}
              onResume={onResume}
              onRemove={onRemove}
              onDetails={onDetails}
              onOpen={onOpen}
              onReveal={onReveal}
            />
          ))
        )}
        <button type="button" className="drop-zone" onClick={onNew}>
          <span className="drop-plus"><DownloadIcon size={16} /></span>
          <span><b>{t("overview.dropTitle")}</b><em>{t("overview.dropHint")}</em></span>
          <span className="drop-hint">HTTP · MAGNET · TORRENT</span>
        </button>
      </section>

      <aside className="overview-side">
        <section className="card mini-card">
          <div className="card-head"><h2>{t("overview.recent")}</h2><span className="kicker">/ {String(counts.completed).padStart(2, "0")}</span><span className="spacer" /><button type="button" className="link-btn" onClick={() => onNavigate("completed")}>{t("overview.viewAll")}</button></div>
          {recent.length === 0 ? <div className="inline-empty compact">{t("overview.recentEmpty")}</div> : recent.map((task) => (
            <div className="mini-line" key={task.id}>
              <span className="mini-check">✓</span>
              <span className="mini-name" title={task.name}>{task.name}</span>
              <span className="mini-value">{formatBytes(task.total ?? task.downloaded)}</span>
            </div>
          ))}
        </section>

        <section className="card mini-card network">
          <div className="card-head"><h2>{t("overview.network")}</h2><span className="spacer" /><HardDriveIcon size={14} /></div>
          <div className="kv-line"><span><WifiIcon size={13} /> {t("overview.dht")}</span><b className={settings.btDht ? "ok" : "muted"}>{settings.btDht ? t("overview.connected") : t("overview.disconnected")}</b></div>
          <div className="kv-line"><span>{t("overview.port")}</span><b>{settings.btListenPort ?? "随机"}</b></div>
          <div className="kv-line"><span>{t("overview.upnp")}</span><b className={settings.btUpnp ? "ok" : "muted"}>{settings.btUpnp ? t("overview.mapped") : "—"}</b></div>
          <div className="kv-line"><span>{t("overview.limit")}</span><b>{settings.globalDownloadLimitKib ? `${Number((settings.globalDownloadLimitKib / 1024).toFixed(2))} MB/s` : t("overview.unlimited")}</b></div>
          <div className="network-note">{uploadSpeed > 0 ? `${formatBytes(uploadSpeed)}/s ↑` : "本地运行 · 仅本机"}</div>
        </section>
      </aside>
    </div>
  );
}
