import {
  createContext,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { zhCN, type MessageKey } from "./zh-CN";

/**
 * 语言注册表。新增语言时：
 * 1. 新建 `en-US.ts` 并按 `MessageKey` 补全；
 * 2. 在这里注册；
 * 3. 在设置页暴露切换入口。
 */
const dictionaries = {
  "zh-CN": zhCN,
} as const satisfies Record<string, Record<MessageKey, string>>;

export type Locale = keyof typeof dictionaries;

type Vars = Record<string, string | number>;

interface I18nValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: MessageKey, vars?: Vars) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>("zh-CN");

  const value = useMemo<I18nValue>(() => {
    const dictionary: Record<MessageKey, string> = dictionaries[locale];

    return {
      locale,
      setLocale,
      t: (key, vars) => {
        let text = dictionary[key] ?? key;
        if (vars) {
          for (const [name, value] of Object.entries(vars)) {
            text = text.split(`{${name}}`).join(String(value));
          }
        }
        return text;
      },
    };
  }, [locale]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n 必须在 <I18nProvider> 内使用");
  }
  return context;
}
