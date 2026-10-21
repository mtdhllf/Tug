import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 与 Rust `AppInfo` 对应。 */
export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  configDir: string;
  dataDir: string;
  downloadDir: string;
}

/** 与 Rust `Settings` 对应。 */
export interface Settings {
  downloadDir: string;
  connectionsPerTask: number;
  maxConcurrentTasks: number;
  globalDownloadLimitKib: number | null;
  /** 仅保留兼容；界面已改用 useSystemProxy。 */
  proxy: string | null;
  useSystemProxy: boolean;
  clipboardWatch: boolean;
  closeToTray: boolean;
  theme: "system" | "light" | "dark";
  locale: string;
  btDht: boolean;
  btListenPort: number | null;
  btUpnp: boolean;
  btUploadLimitKib: number | null;
  btSeedAfterComplete: boolean;
}

export type TaskState =
  | "queued"
  | "fetching_metadata"
  | "downloading"
  | "paused"
  | "seeding"
  | "completed"
  | "failed";

/** 与 Rust `TaskView` 对应。 */
export interface TaskView {
  id: number;
  kind: string;
  name: string;
  state: TaskState;
  total: number | null;
  downloaded: number;
  uploaded: number;
  speedBps: number;
  uploadSpeedBps: number;
  peers: number;
  etaSecs: number | null;
  error: string | null;
  saveDir: string;
  fileName: string | null;
  path: string;
  createdAt: number;
}

export type TaskEvent =
  | { type: "updated"; task: TaskView }
  | { type: "removed"; id: number };

export interface AddTaskInput {
  url: string;
  saveDir?: string | null;
  fileName?: string | null;
  connections?: number | null;
  onlyFiles?: number[] | null;
}

export interface TorrentFileInfo {
  index: number;
  path: string;
  length: number;
  selected: boolean;
}

export interface TorrentPeerInfo {
  address: string;
  state: string;
  client: string | null;
  downloaded: number;
  uploaded: number;
}

export interface TorrentRuntimeDetails {
  peers: TorrentPeerInfo[];
  trackers: string[];
}

export function ping(): Promise<string> {
  return invoke<string>("ping");
}

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

/** 让 Windows 原生标题栏/边框颜色跟随主题。 */
export function setWindowAppearance(dark: boolean): Promise<void> {
  return invoke<void>("set_window_appearance", { dark });
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { settings });
}

/** 读取 Windows 剪贴板中的文件路径（用于 .torrent 文件复制）。 */
export function readClipboardFiles(): Promise<string[]> {
  return invoke<string[]>("clipboard_files");
}

export function listTasks(): Promise<TaskView[]> {
  return invoke<TaskView[]>("list_tasks");
}

export function addTask(input: AddTaskInput): Promise<TaskView> {
  return invoke<TaskView>("add_task", { input });
}

export function pauseTask(id: number): Promise<void> {
  return invoke<void>("pause_task", { id });
}

export function resumeTask(id: number): Promise<TaskView> {
  return invoke<TaskView>("resume_task", { id });
}

export function removeTask(id: number, deleteFiles: boolean): Promise<void> {
  return invoke<void>("remove_task", { id, deleteFiles });
}

export function listTorrentFiles(id: number): Promise<TorrentFileInfo[]> {
  return invoke<TorrentFileInfo[]>("list_torrent_files", { id });
}

export function setTorrentFiles(id: number, files: number[]): Promise<void> {
  return invoke<void>("set_torrent_files", { id, files });
}

export function torrentRuntime(id: number): Promise<TorrentRuntimeDetails> {
  return invoke<TorrentRuntimeDetails>("torrent_runtime", { id });
}

export function onTaskEvent(
  handler: (event: TaskEvent) => void,
): Promise<UnlistenFn> {
  return listen<TaskEvent>("task-event", (event) => handler(event.payload));
}
