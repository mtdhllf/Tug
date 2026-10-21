import type { ReactNode } from "react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/zh-CN";
import {
  ActiveIcon,
  AllIcon,
  CompletedIcon,
  DownloadIcon,
  GridIcon,
  SettingsIcon,
} from "./icons";

export type Filter = "all" | "active" | "completed";
export type View = "overview" | Filter | "settings";

export interface NavCounts {
  all: number;
  active: number;
  completed: number;
}

interface SidebarProps {
  view: View;
  counts: NavCounts;
  downloadDir: string;
  onSelect: (view: View) => void;
}

interface NavItem {
  id: View;
  label: MessageKey;
  icon: (props: { size?: number; className?: string }) => ReactNode;
  count?: keyof NavCounts;
}

const NAV_ITEMS: NavItem[] = [
  { id: "overview", label: "nav.overview", icon: GridIcon },
  { id: "all", label: "nav.all", icon: AllIcon, count: "all" },
  { id: "active", label: "nav.active", icon: ActiveIcon, count: "active" },
  { id: "completed", label: "nav.completed", icon: CompletedIcon, count: "completed" },
];

export function Sidebar({ view, counts, downloadDir, onSelect }: SidebarProps) {
  const { t } = useI18n();

  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">
          <DownloadIcon size={18} />
        </div>
        <div className="brand-text">
          <div className="brand-name">{t("app.name")}</div>
          <div className="brand-tagline">{t("app.tagline")}</div>
        </div>
      </div>

      <div className="nav-label">LIBRARY</div>
      <nav className="nav" aria-label={t("nav.all")}>
        {NAV_ITEMS.map(({ id, label, icon: Icon, count }) => {
          const active = view === id;
          return (
            <button
              key={id}
              type="button"
              className={active ? "nav-item active" : "nav-item"}
              aria-current={active ? "page" : undefined}
              onClick={() => onSelect(id)}
            >
              <Icon size={16} />
              <span className="nav-label-text">{t(label)}</span>
              {count && <span className="nav-count">{String(counts[count]).padStart(2, "0")}</span>}
            </button>
          );
        })}
      </nav>

      <div className="side-storage">
        <div className="side-storage-label">下载目录</div>
        <div className="side-storage-path" title={downloadDir}>{downloadDir || "—"}</div>
        <div className="side-storage-meta">
          <span>{counts.all} 个任务</span>
          <span>{counts.active} 个进行中</span>
        </div>
      </div>

      <div className="sidebar-footer">
        <button
          type="button"
          className={view === "settings" ? "nav-item active" : "nav-item"}
          onClick={() => onSelect("settings")}
        >
          <SettingsIcon size={16} />
          <span className="nav-label-text">{t("nav.settings")}</span>
        </button>
        <div className="sidebar-status">
          <span className="status-dot" />
          ENGINE READY
        </div>
      </div>
    </aside>
  );
}
