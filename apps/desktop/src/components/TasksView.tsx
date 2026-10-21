import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/zh-CN";
import type { TaskView } from "../lib/backend";
import type { Filter, NavCounts, View } from "./Sidebar";
import { TaskTable } from "./TaskTable";
import { PlusIcon } from "./icons";

interface TasksViewProps {
  view: Filter;
  tasks: TaskView[];
  counts: NavCounts;
  query: string;
  sort: "recent" | "speed" | "name";
  selected: Set<number>;
  onSort: (sort: "recent" | "speed" | "name") => void;
  onNavigate: (view: View) => void;
  onToggle: (id: number) => void;
  onToggleAll: () => void;
  onBulkPause: () => void;
  onBulkResume: () => void;
  onBulkRemove: () => void;
  onClearSelection: () => void;
  onPause: (id: number) => void;
  onResume: (id: number) => void;
  onRemove: (task: TaskView) => void;
  onDetails: (task: TaskView) => void;
  onOpen: (task: TaskView) => void;
  onReveal: (task: TaskView) => void;
  onNew: () => void;
}

const EMPTY_DESC: Record<Filter, MessageKey> = {
  all: "empty.desc.all",
  active: "empty.desc.active",
  completed: "empty.desc.completed",
};


export function TasksView({
  view,
  tasks,
  counts,
  query,
  sort,
  selected,
  onSort,
  onNavigate,
  onToggle,
  onToggleAll,
  onBulkPause,
  onBulkResume,
  onBulkRemove,
  onClearSelection,
  onPause,
  onResume,
  onRemove,
  onDetails,
  onOpen,
  onReveal,
  onNew,
}: TasksViewProps) {
  const { t } = useI18n();
  const selectedCount = selected.size;

  return (
    <div className="tasks-view">


      <div className="toolbar">
        <div className="filter-chips">
          {([
            ["all", "tasks.filter.all", counts.all],
            ["active", "tasks.filter.active", counts.active],
            ["completed", "tasks.filter.completed", counts.completed],
          ] as const).map(([id, label, count]) => (
            <button
              key={id}
              type="button"
              className={`chip ${view === id ? "active" : ""}`}
              onClick={() => onNavigate(id)}
            >
              {t(label)} <span>{String(count).padStart(2, "0")}</span>
            </button>
          ))}
        </div>
        <span className="toolbar-spacer" />
        <span className="toolbar-label">排序</span>
        <select
          className="sort-select"
          value={sort}
          onChange={(event) => onSort(event.currentTarget.value as "recent" | "speed" | "name")}
        >
          <option value="recent">{t("tasks.sort.recent")}</option>
          <option value="speed">{t("tasks.sort.speed")}</option>
          <option value="name">{t("tasks.sort.name")}</option>
        </select>
      </div>

      {selectedCount > 0 && (
        <div className="bulkbar">
          <span>{t("tasks.selected", { count: selectedCount })}</span>
          <span className="toolbar-spacer" />
          <button type="button" className="btn btn-sm" onClick={onBulkPause}>{t("tasks.bulk.pause")}</button>
          <button type="button" className="btn btn-sm" onClick={onBulkResume}>{t("tasks.bulk.resume")}</button>
          <button type="button" className="btn btn-sm btn-danger" onClick={onBulkRemove}>{t("tasks.bulk.remove")}</button>
          <button type="button" className="btn btn-sm" onClick={onClearSelection}>{t("tasks.bulk.clear")}</button>
        </div>
      )}

      {tasks.length === 0 ? (
        <div className="empty-state">
          <div className="empty-circle">0</div>
          <h2>{query ? t("tasks.empty") : t(`empty.title`)}</h2>
          <p>{query ? t("tasks.emptyHint") : t(EMPTY_DESC[view])}</p>
          <button type="button" className="btn btn-primary" onClick={onNew}>
            <PlusIcon size={13} /> {t("toolbar.new")}
          </button>
        </div>
      ) : (
        <>
          <TaskTable
            tasks={tasks}
            selected={selected}
            onToggle={onToggle}
            onToggleAll={onToggleAll}
            onPause={onPause}
            onResume={onResume}
            onRemove={onRemove}
            onDetails={onDetails}
            onOpen={onOpen}
            onReveal={onReveal}
          />
          <div className="table-footer">
            <span>{t("tasks.count", { count: tasks.length })}</span>
            <span>按创建时间排序</span>
          </div>
        </>
      )}
    </div>
  );
}
