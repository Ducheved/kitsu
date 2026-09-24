// Translation lookup. Languages load on first use so the startup bundle only
// carries the one in use; English is always present as the fallback for a
// key a translation is missing (the type check makes that a build error, so
// the fallback only matters for a stale cache).

import en, { type Key, type Messages, type Plural } from "./en";

export const LOCALES = {
  en: "English",
  ru: "Русский",
  ja: "日本語",
  de: "Deutsch",
  fr: "Français",
} as const;

export type Locale = keyof typeof LOCALES;

const loaders: Record<Exclude<Locale, "en">, () => Promise<{ default: Messages }>> = {
  ru: () => import("./ru"),
  ja: () => import("./ja"),
  de: () => import("./de"),
  fr: () => import("./fr"),
};

export function systemLocale(): Locale {
  const langs = typeof navigator === "undefined" ? [] : (navigator.languages ?? [navigator.language]);
  for (const l of langs) {
    const base = l.toLowerCase().split("-")[0] as Locale;
    if (base in LOCALES) return base;
  }
  return "en";
}

class I18n {
  locale = $state<Locale>("en");
  messages = $state<Messages>(en);
  private plurals = new Intl.PluralRules("en");

  async use(locale: Locale) {
    const messages = locale === "en" ? en : (await loaders[locale]()).default;
    this.messages = messages;
    this.plurals = new Intl.PluralRules(locale);
    this.locale = locale;
    document.documentElement.lang = locale;
  }

  t = (key: Key, params: Record<string, string | number> = {}): string => {
    let msg: string | Plural = this.messages[key] ?? en[key];
    if (typeof msg !== "string") {
      const n = Number(params.n ?? 0);
      const form = this.plurals.select(n) as keyof Plural;
      msg = msg[form] ?? msg.other;
    }
    return msg.replace(/\{(\w+)\}/g, (_, name: string) => (name in params ? String(params[name]) : `{${name}}`));
  };

  ago = (ms: number, now = Date.now()): string => {
    const s = Math.round((ms - now) / 1000);
    if (Math.abs(s) < 45) return this.t("time.justNow");
    const rtf = new Intl.RelativeTimeFormat(this.locale, { numeric: "auto", style: "short" });
    if (Math.abs(s) < 3600) return rtf.format(Math.round(s / 60), "minute");
    if (Math.abs(s) < 86400) return rtf.format(Math.round(s / 3600), "hour");
    return rtf.format(Math.round(s / 86400), "day");
  };

  list = (items: string[]): string => new Intl.ListFormat(this.locale, { style: "short", type: "conjunction" }).format(items);

  number = (n: number): string => new Intl.NumberFormat(this.locale).format(n);
}

export const i18n = new I18n();
export const t = i18n.t;
export type { Key };
