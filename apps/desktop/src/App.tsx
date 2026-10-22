import { useCallback, useEffect, useMemo, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AddTaskDialog } from "./components/AddTaskDialog";
import { BulkRemoveDialog } from "./components/BulkRemoveDialog";
import { Overview } from "./components/Overview";
import { RemoveTaskDialog } from "./components/RemoveTaskDialog";
import { SettingsPage, type SettingsTab } from "./components/SettingsPage";
import { Sidebar, type Filter, type NavCounts, type View } from "./components/Sidebar";
import { TasksView } from "./components/TasksView";
import { Topbar } from "./components/Topbar";
import { TorrentDetailsDialog } from "./components/TorrentDetailsDialog";
import { useI18n } from "./i18n";
import type { MessageKey } from "./i18n/zh-CN";
import {
  getAppInfo,
  getSettings,
  listTasks,
  onTaskEvent,
  pauseTask,
  ping,
  readClipboardFiles,
  removeTask,
  resumeTask,
  setWindowAppearance,
  type AppInfo,
  type Settings,
  type TaskView,
} from "./lib/backend";
import { formatBytes } from "./lib/format";
import { extractDownloadCandidate, isTorrentPath } from "./lib/downloadLink";
import { platformLabel } from "./lib/platform";
import { DownloadIcon } from "./components/icons";

const TITLES: Record<Filter, MessageKey> = {
  all: "toolbar.title.all",
  active: "toolbar.title.active",
  completed: "toolbar.title.completed",
};

function isFinished(task: TaskView): boolean {
  return task.state === "completed" || task.state === "seeding";
}

function matchesFilter(task: TaskView, filter: Filter): boolean {
  switch (filter) {
    case "active":
      // 下载中列表合并了排队、暂停、失败与进行中任务；
      // 下载完成（含后台共享）统一进入已完成。
      return !isFinished(task);
    case "completed":
      return isFinished(task);
    default:
      return true;
  }
}

function sortTasks(tasks: TaskView[]): TaskView[] {
  return [...tasks].sort((left, right) => right.createdAt - left.createdAt || right.id - left.id);
}

function App() {
  const { t } = useI18n();
  const [view, setView] = useState<View>("overview");
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<"recent" | "speed" | "name">("recent");
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [addOpen, setAddOpen] = useState(false);
  const [addInitialUrl, setAddInitialUrl] = useState<string | null>(null);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("download");
  const [removeTarget, setRemoveTarget] = useState<TaskView | null>(null);
  const [bulkRemoveTargets, setBulkRemoveTargets] = useState<TaskView[] | null>(null);
  const [detailsTarget, setDetailsTarget] = useState<TaskView | null>(null);
  const [clipboardUrl, setClipboardUrl] = useState<string | null>(null);

  const refreshTasks = useCallback(async () => {
    try {
      setTasks(await listTasks());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      await ping();
      const [nextInfo, nextSettings, nextTasks] = await Promise.all([getAppInfo(), getSettings(), listTasks()]);
      setInfo(nextInfo);
      setSettings(nextSettings);
      setTasks(sortTasks(nextTasks));
      setError(null);
    } catch (cause) {
      setInfo(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;

    onTaskEvent((event) => {
      setTasks((previous) => {
        if (event.type === "removed") {
          return previous.filter((task) => task.id !== event.id);
        }
        const exists = previous.some((task) => task.id === event.task.id);
        const next = exists
          ? previous.map((task) => (task.id === event.task.id ? event.task : task))
          : [event.task, ...previous];
        return sortTasks(next);
      });
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });

    const timer = window.setInterval(() => {
      listTasks()
        .then((next) => setTasks(sortTasks(next)))
        .catch(() => {
          /* 事件通道是主路径，轮询失败时保留现有列表。 */
        });
    }, 3000);

    return () => {
      disposed = true;
      unlisten?.();
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (!settings?.clipboardWatch) return;
    let last = "";
    let initialized = false;
    let cancelled = false;

    const poll = async (baseline: boolean) => {
      if (cancelled) return;
      try {
        const [text, files] = await Promise.all([
          readText().catch(() => ""),
          readClipboardFiles().catch(() => [] as string[]),
        ]);
        const fileCandidate = Array.isArray(files) ? files.find(isTorrentPath) : undefined;
        const candidate = fileCandidate ?? extractDownloadCandidate(text ?? "");
        // 启动/开启监视时已有内容只作为基线，不提示；
        // 之后复制的新内容才触发提醒。
        if (baseline || !initialized) {
          last = candidate ?? "";
          initialized = true;
          return;
        }
        if (candidate && candidate !== last) {
          last = candidate;
          setClipboardUrl(candidate);
        }
      } catch {
        initialized = true;
      }
    };

    void poll(true);
    const timer = window.setInterval(() => void poll(false), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [settings?.clipboardWatch]);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type !== "drop") return;
          const torrent = event.payload.paths.find(isTorrentPath);
          if (torrent) {
            setAddInitialUrl(torrent);
            setAddOpen(true);
          }
        })
        .then((stop) => {
          if (disposed) stop();
          else unlisten = stop;
        })
        .catch(() => {
          /* 浏览器预览或非 Tauri 环境没有原生拖拽事件。 */
        });
    } catch {
      /* 浏览器预览或非 Tauri 环境没有原生拖拽事件。 */
    }
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onContextMenu = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Element && target.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
    };
    document.addEventListener("contextmenu", onContextMenu);
    return () => document.removeEventListener("contextmenu", onContextMenu);
  }, []);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = settings?.theme === "dark" || (settings?.theme === "system" && mq.matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      void setWindowAppearance(dark).catch(() => {
        /* 浏览器预览或非 Windows 环境没有原生标题栏可调。 */
      });
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [settings?.theme]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // 去掉 WebView 的刷新/打印快捷键，避免把桌面应用当浏览器用。
      if (event.key === "F5" || ((event.ctrlKey || event.metaKey) && ["r", "p"].includes(event.key.toLowerCase()))) {
        event.preventDefault();
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
        event.preventDefault();
        setAddInitialUrl(null);
        setAddOpen(true);
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        document.getElementById("task-search")?.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const counts = useMemo<NavCounts>(
    () => ({
      all: tasks.length,
      active: tasks.filter((task) => matchesFilter(task, "active")).length,
      completed: tasks.filter(isFinished).length,
    }),
    [tasks],
  );

  const visibleTasks = useMemo(() => {
    if (view === "overview" || view === "settings") return [];
    const base = tasks.filter((task) => matchesFilter(task, view));
    const searched = query.trim()
      ? base.filter((task) => {
          const needle = query.trim().toLowerCase();
          return (
            task.name.toLowerCase().includes(needle) ||
            task.path.toLowerCase().includes(needle) ||
            (task.error ?? "").toLowerCase().includes(needle)
          );
        })
      : base;
    return [...searched].sort((left, right) => {
      if (sort === "speed") return right.speedBps + right.uploadSpeedBps - (left.speedBps + left.uploadSpeedBps);
      if (sort === "name") return left.name.localeCompare(right.name);
      return right.createdAt - left.createdAt || right.id - left.id;
    });
  }, [tasks, view, query, sort]);

  const handlePause = useCallback(async (id: number) => {
    // 先乐观切换状态，避免网络读阻塞时按钮看起来没反应。
    setTasks((previous) =>
      previous.map((task) =>
        task.id === id
          ? { ...task, state: "paused", speedBps: 0, uploadSpeedBps: 0, etaSecs: null }
          : task,
      ),
    );
    try {
      await pauseTask(id);
      await refreshTasks();
    } catch (cause) {
      await refreshTasks();
      window.alert(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refreshTasks]);

  const handleResume = useCallback(async (id: number) => {
    try {
      await resumeTask(id);
      await refreshTasks();
    } catch (cause) {
      window.alert(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refreshTasks]);

  const handleRemove = useCallback(async (task: TaskView, deleteFiles: boolean) => {
    try {
      await removeTask(task.id, deleteFiles);
      setRemoveTarget(null);
      await refreshTasks();
    } catch (cause) {
      window.alert(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refreshTasks]);

  const handleOpen = useCallback(async (task: TaskView) => {
    try {
      await openPath(task.path);
    } catch (cause) {
      window.alert(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const handleReveal = useCallback(async (task: TaskView) => {
    try {
      await revealItemInDir(task.path);
    } catch (cause) {
      window.alert(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  function navigate(next: View) {
    setView(next);
    setSelected(new Set());
  }

  function toggleSelected(id: number) {
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleAll() {
    const all = visibleTasks.every((task) => selected.has(task.id));
    setSelected(all ? new Set() : new Set(visibleTasks.map((task) => task.id)));
  }

  async function bulkPause() {
    const targets = visibleTasks.filter((task) => selected.has(task.id) && ["downloading", "fetching_metadata", "seeding"].includes(task.state));
    const ids = new Set(targets.map((task) => task.id));
    setTasks((previous) =>
      previous.map((task) =>
        ids.has(task.id) ? { ...task, state: "paused", speedBps: 0, uploadSpeedBps: 0, etaSecs: null } : task,
      ),
    );
    await Promise.all(targets.map((task) => pauseTask(task.id).catch(() => undefined)));
    setSelected(new Set());
    await refreshTasks();
  }

  async function bulkResume() {
    const targets = visibleTasks.filter((task) => selected.has(task.id) && ["paused", "failed"].includes(task.state));
    await Promise.all(targets.map((task) => resumeTask(task.id).catch(() => undefined)));
    setSelected(new Set());
    await refreshTasks();
  }

  function requestBulkRemove() {
    const targets = visibleTasks.filter((task) => selected.has(task.id));
    if (targets.length === 0) return;
    // 只打开确认弹层；此时绝不能删除或清空列表。
    setBulkRemoveTargets(targets);
  }

  async function confirmBulkRemove(deleteFiles: boolean) {
    const targets = bulkRemoveTargets;
    if (!targets?.length) return;
    for (const task of targets) {
      await removeTask(task.id, deleteFiles).catch(() => undefined);
    }
    setBulkRemoveTargets(null);
    setSelected(new Set());
    await refreshTasks();
  }

  const pageTitle =
    view === "overview" ? t("nav.overview") :
    view === "settings" ? t("settings.title") :
    t(TITLES[view]);

  const pageSubtitle =
    view === "overview"
      ? `${counts.active} 个进行中 · ${formatBytes(tasks.reduce((sum, task) => sum + task.speedBps, 0))}/s · DHT ${settings?.btDht ? "已连接" : "未启用"}`
      : view === "settings"
        ? "下载、网络、BitTorrent 与外观"
        : `${visibleTasks.length} 个任务 · ${counts.active} 个进行中`;

  const statusText = error ? t("status.offline") : loading ? t("diag.checking") : t("status.ready");

  return (
    <div className="app">
      <Sidebar view={view} counts={counts} downloadDir={settings?.downloadDir ?? ""} onSelect={navigate} />

      <main className="main">
        <Topbar
          title={pageTitle}
          subtitle={pageSubtitle}
          search={query}
          showSearch={view !== "settings"}
          onSearch={(value) => {
            setQuery(value);
            if (value && (view === "overview" || view === "settings")) setView("all");
          }}
          onNew={() => {
            setAddInitialUrl(null);
            setAddOpen(true);
          }}
        />

        <div
          className="content"
          onDragOver={(event) => {
            event.preventDefault();
          }}
          onDrop={(event) => {
            event.preventDefault();
            const raw =
              event.dataTransfer.getData("text/uri-list") ||
              event.dataTransfer.getData("text/plain");
            const text = raw.split(/\r?\n/).map((line) => line.trim()).find(Boolean);
            if (text && /^(https?:\/\/|magnet:|.*\.torrent$)/i.test(text)) {
              setAddInitialUrl(text);
              setAddOpen(true);
            }
          }}
        >
          {error && !info ? (
            <div className="error-state">
              <div className="error-panel">
                <div className="error-panel-icon"><DownloadIcon size={20} /></div>
                <div>
                  <h2>连接不到下载引擎。</h2>
                  <p>前端已加载，但 Tauri IPC 没有响应。任务数据尚未丢失。</p>
                </div>
                <span className="tag fail">IPC_TIMEOUT</span>
              </div>
              <div className="error-details">
                <div><span>错误</span><code>{error}</code></div>
                <div><span>配置目录</span><code>%APPDATA%\\Tug</code></div>
                <div><span>最近心跳</span><code>{new Date().toLocaleTimeString()}</code></div>
              </div>
              <div className="error-actions">
                <button type="button" className="btn btn-primary" onClick={() => void refresh()}>重新连接</button>
                <button type="button" className="btn" onClick={() => void refresh()}>查看诊断</button>
              </div>
            </div>
          ) : loading && !settings ? (
            <div className="loading-state"><div className="spinner" /> 正在读取任务…</div>
          ) : view === "overview" && settings ? (
            <Overview
              tasks={tasks}
              counts={counts}
              settings={settings}
              info={info}
              onQuickAdd={(url) => {
                setAddInitialUrl(url);
                setAddOpen(true);
              }}
              onNew={() => {
                setAddInitialUrl(null);
                setAddOpen(true);
              }}
              onNavigate={navigate}
              onPause={handlePause}
              onResume={handleResume}
              onRemove={setRemoveTarget}
              onDetails={setDetailsTarget}
              onOpen={handleOpen}
              onReveal={handleReveal}
            />
          ) : view === "settings" && settings ? (
            <SettingsPage
              settings={settings}
              info={info}
              tab={settingsTab}
              onTab={setSettingsTab}
              onSaved={setSettings}
            />
          ) : settings ? (
            <TasksView
              view={view as Filter}
              tasks={visibleTasks}
              counts={counts}
              query={query}
              sort={sort}
              selected={selected}
              onSort={setSort}
              onNavigate={navigate}
              onToggle={toggleSelected}
              onToggleAll={toggleAll}
              onBulkPause={() => void bulkPause()}
              onBulkResume={() => void bulkResume()}
              onBulkRemove={requestBulkRemove}
              onClearSelection={() => setSelected(new Set())}
              onPause={handlePause}
              onResume={handleResume}
              onRemove={setRemoveTarget}
              onDetails={setDetailsTarget}
              onOpen={handleOpen}
              onReveal={handleReveal}
              onNew={() => {
                setAddInitialUrl(null);
                setAddOpen(true);
              }}
            />
          ) : null}

          {clipboardUrl && !addOpen && (
            <div className="clipboard-toast">
              <div className="clipboard-toast-head"><span className="status-dot" /> 剪贴板检测到链接</div>
              <div className="clipboard-toast-body">
                <code title={clipboardUrl}>{clipboardUrl}</code>
                <div className="clipboard-toast-actions">
                  <button type="button" className="btn btn-sm" onClick={() => setClipboardUrl(null)}>忽略</button>
                  <button type="button" className="btn btn-sm btn-primary" onClick={() => { setAddInitialUrl(clipboardUrl); setAddOpen(true); setClipboardUrl(null); }}>新建下载</button>
                </div>
              </div>
            </div>
          )}
        </div>

        <footer className="statusbar">
          <span className="status-left"><span className={`status-dot ${error ? "error" : loading ? "pending" : "ok"}`} />{statusText}</span>
          <span>DHT {settings?.btDht ? "CONNECTED" : "OFF"}</span>
          <span>PORT {settings?.btListenPort ?? "AUTO"}</span>
          <span>UPnP {settings?.btUpnp ? "MAPPED" : "OFF"}</span>
          <span className="statusbar-right">{info ? `${info.name} v${info.version} · ${platformLabel(info.platform)}` : t("status.m0")}</span>
        </footer>
      </main>

      <AddTaskDialog
        visible={addOpen}
        defaultDir={settings?.downloadDir ?? ""}
        defaultConnections={settings?.connectionsPerTask ?? 8}
        initialUrl={addInitialUrl}
        onClose={() => { setAddOpen(false); setAddInitialUrl(null); }}
        onAdded={(task) => {
          setTasks((previous) => sortTasks([task, ...previous.filter((item) => item.id !== task.id)]));
          setAddInitialUrl(null);
          void refreshTasks();
          if (view === "overview") setView("all");
        }}
      />

      {removeTarget && <RemoveTaskDialog task={removeTarget} onClose={() => setRemoveTarget(null)} onConfirm={handleRemove} />}

      {bulkRemoveTargets && (
        <BulkRemoveDialog
          tasks={bulkRemoveTargets}
          onClose={() => setBulkRemoveTargets(null)}
          onConfirm={confirmBulkRemove}
        />
      )}

      {detailsTarget && (
        <TorrentDetailsDialog task={tasks.find((task) => task.id === detailsTarget.id) ?? detailsTarget} onClose={() => setDetailsTarget(null)} />
      )}
    </div>
  );
}

export default App;
