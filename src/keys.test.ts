import { describe, expect, test } from "vitest";
import { initialState, reduce, reconcile, type UiState } from "./keys";
import type { ScoredCard } from "./fuzzy";
import type { SessionCard } from "./render";

function card(paneId: string): SessionCard {
  return {
    paneId,
    tmuxSession: "s",
    windowIndex: 0,
    paneIndex: 0,
    title: paneId,
    summary: "",
    state: "Idle",
    idleSecs: 0,
  };
}
const scored = (ids: string[]): ScoredCard[] =>
  ids.map((id) => ({ card: card(id), field: null, positions: [], score: 0 }));

const key = (k: string, over: Partial<{ ctrl: boolean; composing: boolean }> = {}) =>
  ({ type: "key", key: k, ctrl: false, composing: false, ...over }) as const;

const normalAt = (id: string): UiState => ({ mode: "normal", query: "", selectedPaneId: id });

describe("normal mode", () => {
  const vis = scored(["%1", "%2", "%3"]);

  test("j moves down, k moves up, clamped at both ends", () => {
    let s = normalAt("%1");
    s = reduce(s, key("j"), vis).state;
    expect(s.selectedPaneId).toBe("%2");
    s = reduce(s, key("k"), vis).state;
    expect(s.selectedPaneId).toBe("%1");
    s = reduce(s, key("k"), vis).state;
    expect(s.selectedPaneId).toBe("%1"); // clamped top
    s = reduce(normalAt("%3"), key("j"), vis).state;
    expect(s.selectedPaneId).toBe("%3"); // clamped bottom
  });

  test("g selects first, G selects last", () => {
    expect(reduce(normalAt("%2"), key("g"), vis).state.selectedPaneId).toBe("%1");
    expect(reduce(normalAt("%2"), key("G"), vis).state.selectedPaneId).toBe("%3");
  });

  test("/ enters search mode with empty query", () => {
    const { state } = reduce(normalAt("%2"), key("/"), vis);
    expect(state.mode).toBe("search");
    expect(state.query).toBe("");
  });

  test("Enter jumps to the selected card", () => {
    const { action } = reduce(normalAt("%2"), key("Enter"), vis);
    expect(action).toEqual({ type: "jump", card: vis[1].card });
  });

  test("Escape hides", () => {
    expect(reduce(normalAt("%1"), key("Escape"), vis).action).toEqual({ type: "hide" });
  });

  test("enter_with_no_matches_is_noop", () => {
    const s = normalAt("%1");
    const { state, action } = reduce(s, key("Enter"), []);
    expect(action).toEqual({ type: "none" });
    expect(state).toEqual(s);
  });

  test("enter_ignored_while_composing", () => {
    const { action } = reduce(normalAt("%1"), key("Enter", { composing: true }), vis);
    expect(action).toEqual({ type: "none" });
  });
});

describe("search mode", () => {
  const vis = scored(["%1", "%2"]);
  const searching: UiState = { mode: "search", query: "ra", selectedPaneId: "%1" };

  test("Escape clears query and returns to normal", () => {
    const { state } = reduce(searching, key("Escape"), vis);
    expect(state).toEqual({ mode: "normal", query: "", selectedPaneId: "%1" });
  });

  test("Enter jumps to selected match", () => {
    expect(reduce(searching, key("Enter"), vis).action).toEqual({
      type: "jump",
      card: vis[0].card,
    });
  });

  test("Backspace on empty query returns to normal", () => {
    const empty: UiState = { ...searching, query: "" };
    expect(reduce(empty, key("Backspace"), vis).state.mode).toBe("normal");
    // with a non-empty query the input owns Backspace: state unchanged
    expect(reduce(searching, key("Backspace"), vis).state).toEqual(searching);
  });

  test("query event sets query and selects top match", () => {
    const { state } = reduce(searching, { type: "query", value: "rai" }, vis);
    expect(state.query).toBe("rai");
    expect(state.selectedPaneId).toBe("%1");
  });

  test("ctrl+j moves selection while typing", () => {
    const { state } = reduce(searching, key("j", { ctrl: true }), vis);
    expect(state.selectedPaneId).toBe("%2");
  });

  test("plain j is NOT navigation in search mode", () => {
    const { state, action } = reduce(searching, key("j"), vis);
    expect(state).toEqual(searching);
    expect(action).toEqual({ type: "none" });
  });
});

describe("handled flag (drives preventDefault in the glue)", () => {
  const vis = scored(["%1", "%2"]);
  const searching: UiState = { mode: "search", query: "ra", selectedPaneId: "%1" };

  test("backspace with text is NOT handled (input owns deletion)", () => {
    expect(reduce(searching, key("Backspace"), vis).handled).toBe(false);
    expect(reduce({ ...searching, query: "" }, key("Backspace"), vis).handled).toBe(true);
  });

  test("navigation and action keys are handled; unbound keys are not", () => {
    expect(reduce(normalAt("%1"), key("j"), vis).handled).toBe(true);
    expect(reduce(normalAt("%1"), key("Enter"), vis).handled).toBe(true);
    expect(reduce(normalAt("%1"), key("/"), vis).handled).toBe(true);
    expect(reduce(normalAt("%1"), key("x"), vis).handled).toBe(false);
    // plain j in search belongs to the input
    expect(reduce(searching, key("j"), vis).handled).toBe(false);
    expect(reduce(searching, key("j", { ctrl: true }), vis).handled).toBe(true);
    // composing keys are never handled
    expect(reduce(normalAt("%1"), key("Enter", { composing: true }), vis).handled).toBe(false);
  });
});

describe("shown / reconcile", () => {
  test("shown resets dirty state to normal, empty query, first card", () => {
    const dirty: UiState = { mode: "search", query: "xx", selectedPaneId: "%9" };
    const { state } = reduce(dirty, { type: "shown" }, scored(["%1", "%2"]));
    expect(state).toEqual({ mode: "normal", query: "", selectedPaneId: "%1" });
  });

  test("shown with empty list selects null", () => {
    expect(reduce(initialState, { type: "shown" }, []).state.selectedPaneId).toBeNull();
  });

  test("reconcile keeps selection when still visible", () => {
    const s = normalAt("%2");
    expect(reconcile(s, scored(["%1", "%2"]), 1)).toEqual(s);
  });

  test("reconcile_clamps_when_selected_disappears", () => {
    const s = normalAt("%3");
    // %3 was at index 2; it vanished and only 2 cards remain → clamp to last
    expect(reconcile(s, scored(["%1", "%2"]), 2).selectedPaneId).toBe("%2");
    // old index still in range → same index
    expect(reconcile(normalAt("%2"), scored(["%a", "%b", "%c"]), 1).selectedPaneId).toBe("%b");
    // empty list → null
    expect(reconcile(s, [], 2).selectedPaneId).toBeNull();
  });
});
