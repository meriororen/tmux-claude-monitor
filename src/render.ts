// Pure HTML renderer for the panel (vitest-testable without a DOM).

import type { ScoredCard } from "./fuzzy";
import type { UiState } from "./keys";

export type CardState =
  | "Working"
  | "NeedsInputStructured"
  | "NeedsInputProse"
  | "Idle"
  | "Stale"
  | "Dead";

export interface SessionCard {
  paneId: string;
  tmuxSession: string;
  windowIndex: number;
  paneIndex: number;
  title: string;
  summary: string;
  state: CardState;
  idleSecs: number;
  sessionUuid: string | null;
  cwd: string;
  starred: boolean;
  transcriptMissing: boolean;
}

const escChar = (c: string): string => {
  switch (c) {
    case "&":
      return "&amp;";
    case "<":
      return "&lt;";
    case ">":
      return "&gt;";
    case '"':
      return "&quot;";
    default:
      return c;
  }
};

const esc = (s: string): string => Array.from(s).map(escChar).join("");

// Escape-then-wrap per code point: an escaped entity is emitted atomically,
// so a highlight can never split "&lt;" in half.
function escapeWithHighlights(text: string, positions: number[]): string {
  const marked = new Set(positions);
  return Array.from(text)
    .map((c, i) => (marked.has(i) ? `<span class="hl">${escChar(c)}</span>` : escChar(c)))
    .join("");
}

export function formatIdle(secs: number): string {
  if (secs < 60) return "now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h`;
}

function dotClass(state: CardState): string {
  switch (state) {
    case "Working":
      return "work";
    case "NeedsInputStructured":
    case "NeedsInputProse":
      return "wait";
    case "Stale":
      return "stale";
    case "Idle":
      return "idle";
    case "Dead":
      return "dead";
  }
}

export function abbrevHome(p: string): string {
  return p.replace(/^\/Users\/[^/]+\//, "~/");
}

const needsInput = (s: CardState) => s === "NeedsInputStructured" || s === "NeedsInputProse";

const HINTS = {
  normal:
    "<kbd>j/k</kbd> move · <kbd>g/G</kbd> top/bottom · <kbd>/</kbd> search · <kbd>↵</kbd> jump · <kbd>esc</kbd> hide · <kbd>f</kbd> ★ · <kbd>⇥</kbd> tab",
  search:
    "<kbd>type</kbd> filter · <kbd>↵</kbd> jump to selected · <kbd>esc</kbd> clear → normal · <kbd>^j/^k</kbd> move",
} as const;

export function renderCards(visible: ScoredCard[], ui: UiState, total: number, favCount: number): string {
  const inSearch = ui.mode === "search";
  const count = inSearch ? `${visible.length}/${total}` : `${total}`;
  const header = `<div class="phead" data-tauri-drag-region>
    <span data-tauri-drag-region>CLAUDE SESSIONS — ${count}</span>
    <button class="gear" id="gear" title="Settings">⚙</button>
  </div>`;
  // NOTE: the searchbar is deliberately NOT part of this output. It lives in
  // a separate, persistent container (see main.ts) so the 2s refresh and
  // per-keystroke renders never recreate the <input> — recreating it would
  // destroy IME composition, caret, and focus.
  const body = visible
    .map((sc) => {
      const c = sc.card;
      const attn = needsInput(c.state) ? " attn" : "";
      const sel = c.paneId === ui.selectedPaneId ? " selected" : "";
      const dead = c.state === "Dead";
      const idle =
        c.state === "Working" ? "now" : dead && c.transcriptMissing ? "—" : formatIdle(c.idleSecs);
      const title =
        sc.field === "title" ? escapeWithHighlights(c.title, sc.positions) : esc(c.title);
      const summary = dead
        ? esc(c.transcriptMissing ? "transcript missing" : `${abbrevHome(c.cwd)} · resume ↻`)
        : sc.field === "summary"
          ? escapeWithHighlights(c.summary, sc.positions)
          : esc(c.summary);
      const star = `<button class="star${c.starred ? " on" : ""}" title="favorite">${c.starred ? "★" : "☆"}</button>`;
      const tag = inSearch
        ? `<span class="stag">${
            sc.field === "session"
              ? escapeWithHighlights(c.tmuxSession, sc.positions)
              : esc(c.tmuxSession)
          }</span>`
        : "";
      return `<div class="sess${attn}${sel}${dead ? " dead" : ""}" data-session="${esc(c.tmuxSession)}" data-window="${c.windowIndex}" data-pane="${c.paneIndex}" data-uuid="${esc(c.sessionUuid ?? "")}" data-cwd="${esc(c.cwd)}" data-title="${esc(c.title)}" data-missing="${c.transcriptMissing}">
        <div class="srow"><div class="dot ${dotClass(c.state)}"></div><div class="stitle">${title}</div>${tag}${star}<div class="smeta">${idle}</div></div>
        <div class="ssummary">${summary}</div>
      </div>`;
    })
    .join("\n");
  const empty =
    visible.length === 0
      ? `<div class="empty" data-tauri-drag-region>${
          inSearch
            ? "No matches"
            : ui.tab === "favorites"
              ? "No favorites yet — press f on a session"
              : "No Claude sessions found in tmux"
        }</div>`
      : "";
  const chip = (tab: "all" | "favorites", label: string) =>
    `<button class="chip${ui.tab === tab ? " active" : ""}" data-tab="${tab}">${label}</button>`;
  const tabs = `<div class="tabs" data-tauri-drag-region>${chip("all", "All")}${chip("favorites", `Favorites ${favCount}`)}</div>`;
  const hints = `<div class="hintbar" data-tauri-drag-region>${HINTS[ui.mode]}</div>`;
  return header + body + empty + tabs + hints;
}

/// Rendered once per entry into search mode, into its own container.
export function renderSearchBar(query: string): string {
  return `<div class="searchbar"><span class="slash">/</span><input id="q" type="text" value="${esc(query)}" autocomplete="off" spellcheck="false" /></div>`;
}
