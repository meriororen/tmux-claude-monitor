// Modal (vim-style) keyboard state machine — pure and DOM-free.
// The DOM layer feeds it events; it answers with the next state and an
// action. Printable text in search mode is owned by the real <input>
// (IME-safe); only its resulting value arrives here as a "query" event.

import type { ScoredCard } from "./fuzzy";
import type { SessionCard } from "./render";

export type Mode = "normal" | "search";

export interface UiState {
  mode: Mode;
  query: string;
  selectedPaneId: string | null;
}

export const initialState: UiState = { mode: "normal", query: "", selectedPaneId: null };

export type UiEvent =
  | { type: "key"; key: string; ctrl: boolean; composing: boolean }
  | { type: "query"; value: string }
  | { type: "shown" };

export type Action = { type: "none" } | { type: "jump"; card: SessionCard } | { type: "hide" };

const NONE: Action = { type: "none" };

function indexOfSelected(state: UiState, visible: ScoredCard[]): number {
  return visible.findIndex((s) => s.card.paneId === state.selectedPaneId);
}

function selectAt(state: UiState, visible: ScoredCard[], index: number): UiState {
  if (visible.length === 0) return { ...state, selectedPaneId: null };
  const clamped = Math.min(Math.max(index, 0), visible.length - 1);
  return { ...state, selectedPaneId: visible[clamped].card.paneId };
}

function move(state: UiState, visible: ScoredCard[], delta: number): UiState {
  const i = indexOfSelected(state, visible);
  return selectAt(state, visible, i < 0 ? 0 : i + delta);
}

function jumpSelected(state: UiState, visible: ScoredCard[]): Action {
  const i = indexOfSelected(state, visible);
  const target = i >= 0 ? visible[i] : visible[0];
  return target ? { type: "jump", card: target.card } : NONE;
}

export function reduce(
  state: UiState,
  ev: UiEvent,
  visible: ScoredCard[],
): { state: UiState; action: Action; handled: boolean } {
  if (ev.type === "shown") {
    return { state: selectAt({ ...initialState }, visible, 0), action: NONE, handled: true };
  }
  if (ev.type === "query") {
    return { state: selectAt({ ...state, query: ev.value }, visible, 0), action: NONE, handled: true };
  }
  // IME composition: the input owns every keystroke until composition ends.
  if (ev.composing) return { state, action: NONE, handled: false };

  const { key, ctrl } = ev;
  if (state.mode === "normal") {
    switch (key) {
      case "j":
      case "ArrowDown":
        return { state: move(state, visible, 1), action: NONE, handled: true };
      case "k":
      case "ArrowUp":
        return { state: move(state, visible, -1), action: NONE, handled: true };
      case "g":
        return { state: selectAt(state, visible, 0), action: NONE, handled: true };
      case "G":
        return { state: selectAt(state, visible, visible.length - 1), action: NONE, handled: true };
      case "/":
        return { state: { ...state, mode: "search", query: "" }, action: NONE, handled: true };
      case "Enter":
        return { state, action: jumpSelected(state, visible), handled: true };
      case "Escape":
        return { state, action: { type: "hide" }, handled: true };
      default:
        return { state, action: NONE, handled: false };
    }
  }
  // search mode
  switch (key) {
    case "Escape":
      return { state: { ...state, mode: "normal", query: "" }, action: NONE, handled: true };
    case "Enter":
      return { state, action: jumpSelected(state, visible), handled: true };
    case "Backspace":
      if (state.query === "") {
        return { state: { ...state, mode: "normal" }, action: NONE, handled: true };
      }
      return { state, action: NONE, handled: false }; // input owns deletion
    case "j":
    case "ArrowDown":
      if (ctrl || key === "ArrowDown")
        return { state: move(state, visible, 1), action: NONE, handled: true };
      return { state, action: NONE, handled: false };
    case "k":
    case "ArrowUp":
      if (ctrl || key === "ArrowUp")
        return { state: move(state, visible, -1), action: NONE, handled: true };
      return { state, action: NONE, handled: false };
    default:
      return { state, action: NONE, handled: false };
  }
}

/// After a data refresh: keep the selection if its pane is still visible;
/// otherwise take the card now sitting at the selection's previous index
/// (clamped to the new last), or null when nothing is visible.
export function reconcile(state: UiState, visible: ScoredCard[], prevIndex: number): UiState {
  if (state.selectedPaneId !== null && indexOfSelected(state, visible) >= 0) return state;
  return selectAt(state, visible, prevIndex);
}
