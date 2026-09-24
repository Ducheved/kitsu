import type { CheckStatus, RunState } from "./types";

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

export function duration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
}

export function short(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 7) : "";
}

/** One word for a check's state, and how to color it. */
export function checkWord(s: CheckStatus): { word: string; tone: "ok" | "bad" | "warn" | "dim"; hint?: string } {
  switch (s.status) {
    case "current":
      return s.outcome === "pass" ? { word: "passes", tone: "ok" } : { word: s.outcome === "fail" ? "fails" : s.outcome, tone: "bad" };
    case "carried":
      return s.outcome === "pass"
        ? { word: "passes", tone: "ok", hint: "unchanged since it last ran" }
        : { word: "fails", tone: "bad", hint: "unchanged since it last ran" };
    case "stale":
      return { word: "stale", tone: "warn", hint: s.changed.length ? `changed since: ${s.changed.join(", ")}${s.more ? ` +${s.more}` : ""}` : "the tree changed" };
    case "unverified":
      return { word: "not run", tone: "dim" };
    case "missing":
      return { word: "undefined", tone: "bad", hint: "no such check in kitsu.toml" };
  }
}

export function runWord(state: RunState, stop: string | null): string {
  if (state === "finished") return stop === "cancelled" ? "stopped" : stop === "end_turn" || !stop ? "finished" : `finished (${stop.replace(/_/g, " ")})`;
  return state;
}
