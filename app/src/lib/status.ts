// Structured state from the Rust side, said in the person's language.

import { i18n, t } from "./i18n/index.svelte";
import type { CheckStatus, DigestItem, RunState, TaskView, Usage } from "./types";

export function statusText(v: TaskView): string {
  const s = v.status;
  const more = v.others ? t("status.others", { n: v.others }) : "";
  switch (s.kind) {
    case "running":
      return t(s.stopping ? "status.stopping" : "status.running", { agent: s.agent }) + more;
    case "asking":
      return t("status.asking", { agent: s.agent, n: s.asks });
    case "review":
      switch (s.verdict) {
        case "verified":
          return t("status.verified") + more;
        case "failing":
          return t("status.failing", { checks: i18n.list(s.failing) }) + more;
        case "unverified":
          return t("status.unverified") + more;
        case "empty":
          return t("status.empty", { agent: s.agent }) + more;
        default:
          return t("status.unknown") + more;
      }
    case "failed":
      return t("status.failed", { agent: s.agent, detail: s.detail });
    case "interrupted":
      return t("status.interrupted");
    case "blocked_by_question":
      return t("status.blockedQuestion", { questions: i18n.list(s.questions) });
    case "blocked_by_tasks":
      return t("status.blockedTasks", { tasks: i18n.list(s.tasks) });
    case "ready":
      return t("status.ready");
    case "done":
    case "dropped":
      return t(s.kind === "done" ? "status.done" : "status.dropped") + (v.others ? t("status.leftover", { n: v.others }) : "");
  }
}

function outcomeWord(outcome: string): string {
  if (outcome === "pass" || outcome === "fail" || outcome === "timeout" || outcome === "error") return t(`outcome.${outcome}`);
  return outcome;
}

export function digestText(item: DigestItem): string {
  const p = item.params ?? {};
  switch (item.kind) {
    case "ask":
      return t("digest.ask", { title: p.title ?? "" });
    case "failed":
      return t("digest.failed", { detail: p.detail ?? "" });
    case "interrupted":
      return t("digest.interrupted");
    case "check":
      return t(p.on === "run" ? "digest.checkRun" : "digest.checkCheckout", { check: p.check, outcome: outcomeWord(p.outcome) });
    case "accepted":
      return t("digest.accepted");
    case "protocol":
      return t("digest.protocol");
    case "finished":
      if (p.stop_reason === "end_turn" || !p.stop_reason) return t("digest.finished");
      if (p.stop_reason === "cancelled") return t("digest.cancelled");
      return t("digest.stopped", { reason: String(p.stop_reason).replace(/_/g, " ") });
    default:
      return item.text;
  }
}

/** One word for a check's state, and how to color it. */
export function checkWord(s: CheckStatus): { word: string; tone: "ok" | "bad" | "warn" | "dim"; hint?: string } {
  switch (s.status) {
    case "current":
      return s.outcome === "pass" ? { word: t("check.passes"), tone: "ok" } : { word: outcomeWord(s.outcome), tone: "bad" };
    case "carried":
      return s.outcome === "pass"
        ? { word: t("check.passes"), tone: "ok", hint: t("check.unchanged") }
        : { word: t("check.fails"), tone: "bad", hint: t("check.unchanged") };
    case "stale":
      return {
        word: t("check.stale"),
        tone: "warn",
        hint: s.changed.length ? t("check.changedSince", { files: s.changed.join(", ") + (s.more ? ` +${s.more}` : "") }) : t("check.treeChanged"),
      };
    case "unverified":
      return { word: t("check.notRun"), tone: "dim" };
    case "missing":
      return { word: t("check.undefined"), tone: "bad", hint: t("check.missing") };
  }
}

export function runWord(state: RunState, stop: string | null, resolution?: string | null): string {
  if (resolution === "accepted") return t("runWord.accepted");
  if (resolution === "discarded") return t("runWord.discarded");
  if (state === "finished") {
    if (stop === "cancelled") return t("runWord.stopped");
    if (!stop || stop === "end_turn") return t("runWord.finished");
    return t("runWord.finishedWith", { reason: stop.replace(/_/g, " ") });
  }
  return t(`runWord.${state}`);
}

/**
 * Parameters for a "{tokens} tokens" message. Compact numbers ("1.5K",
 * "1,5 тыс.") take the plural of a round thousand, which is what they read
 * as; smaller ones take their own.
 */
export function tokenParams(n: number): { tokens: string; n: number } {
  return { tokens: i18n.compact(n), n: n >= 1000 ? 1000 : n };
}

/** Tokens a run spent, as far as its agent said. */
export function spent(u: Usage | null | undefined): number | null {
  if (!u) return null;
  if (u.total != null) return u.total;
  if (u.input == null && u.output == null) return null;
  return (u.input ?? 0) + (u.output ?? 0);
}

/** " · 1.5K tokens · $0.02 · 3% of its context window", or a note that nothing was reported. */
export function usageText(u: Usage | null | undefined): string {
  const n = spent(u);
  let out = n == null ? "" : t("act.tokens", tokenParams(n));
  if (u?.cost != null && u.currency) out += ` · ${i18n.money(u.cost, u.currency)}`;
  if (u?.context_used != null && u.context_size) out += t("act.context", { pct: i18n.percent(u.context_used / u.context_size) });
  return out || t("act.silent");
}

/** Rough token count for text Kitsu wrote itself (briefs). */
export function estimateTokens(text: string): number {
  return Math.ceil(new TextEncoder().encode(text).length / 4);
}

export function duration(ms: number): string {
  const f = (n: number, unit: "millisecond" | "second" | "minute") =>
    new Intl.NumberFormat(i18n.locale, { style: "unit", unit, unitDisplay: "narrow", maximumFractionDigits: 1 }).format(n);
  if (ms < 1000) return f(ms, "millisecond");
  if (ms < 60_000) return f(ms / 1000, "second");
  return `${f(Math.floor(ms / 60_000), "minute")} ${f(Math.round((ms % 60_000) / 1000), "second")}`;
}
