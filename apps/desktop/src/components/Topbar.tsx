import { useI18n } from "../i18n";
import { PlusIcon, SearchIcon } from "./icons";

interface TopbarProps {
  title: string;
  subtitle: string;
  search: string;
  showSearch: boolean;
  onSearch: (value: string) => void;
  onNew: () => void;
}

export function Topbar({
  title,
  subtitle,
  search,
  showSearch,
  onSearch,
  onNew,
}: TopbarProps) {
  const { t } = useI18n();

  return (
    <header className="topbar">
      <div className="page-title">
        <h1>{title}</h1>
        <p>{subtitle}</p>
      </div>
      <div className="topbar-spacer" />
      {showSearch && (
        <label className="search">
          <SearchIcon size={14} />
          <input
            id="task-search"
            type="search"
            value={search}
            onChange={(event) => onSearch(event.currentTarget.value)}
            placeholder={t("topbar.search")}
            spellCheck={false}
          />
          <kbd>Ctrl K</kbd>
        </label>
      )}
      <button type="button" className="btn btn-primary" onClick={onNew}>
        <PlusIcon size={14} />
        {t("toolbar.new")}
        <span className="btn-kbd">Ctrl N</span>
      </button>
    </header>
  );
}
