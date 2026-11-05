import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { renderCards, renderSearchBar, type SessionCard } from "./render";
import { filterCards, type ScoredCard } from "./fuzzy";
import { initialState, reduce, reconcile, type Action, type UiState } from "./keys";

// Two containers: #bar holds the search <input> and is written ONLY on mode
// transitions (recreating a live input would destroy IME composition, caret
// and focus); #list is rewritten freely on every state/data change.
const app = document.getElementById("app")!;
// data-tauri-drag-region: Tauri's handler only honors the attribute on the
// exact mousedown target, so cards (and their children) stay clickable while
// any empty surface — gaps, header, hint bar — drags the window.
app.innerHTML = `<div id="bar" data-tauri-drag-region></div><div id="list" data-tauri-drag-region></div>`;
const bar = document.getElementById("bar")!;
const list = document.getElementById("list")!;

let ui: UiState = initialState;
let cards: SessionCard[] = [];
let visible: ScoredCard[] = [];

function showToast(msg: string): void {
  const t = document.createElement("div");
  t.className = "toast";
  t.textContent = msg;
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 4000);
}

function jump(card: SessionCard): void {
  // Backend hides the panel after a successful switch (jump always hides).
  invoke("jump_to_session", {
    session: card.tmuxSession,
    windowIndex: card.windowIndex,
    paneIndex: card.paneIndex,
  }).catch((err) => showToast(String(err)));
}

function apply(action: Action): void {
  if (action.type === "jump") jump(action.card);
  else if (action.type === "hide") invoke("hide_panel").catch(() => {});
}

let renderedMode: UiState["mode"] = "normal";

function render(): void {
  visible = filterCards(ui.query, cards);
  // Searchbar container: touched only when the mode actually changes.
  if (ui.mode !== renderedMode) {
    if (ui.mode === "search") {
      bar.innerHTML = renderSearchBar(ui.query);
      const input = document.getElementById("q") as HTMLInputElement;
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    } else {
      bar.innerHTML = "";
    }
    renderedMode = ui.mode;
  }
  list.innerHTML = renderCards(visible, ui, cards.length);
  // Keep the keyboard selection on-screen; "nearest" is a no-op when it
  // already is, so the 2s refresh doesn't yank a hand-scrolled list around.
  list.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
}

function dispatch(ev: Parameters<typeof reduce>[1], withVisible?: ScoredCard[]): boolean {
  const out = reduce(ui, ev, withVisible ?? visible);
  const changed = out.state !== ui;
  ui = out.state;
  if (changed) render();
  apply(out.action);
  return out.handled;
}

// WebKit (Tauri's engine on macOS) fires compositionend BEFORE the keydown of
// the Enter that commits an IME conversion, so that keydown arrives with
// isComposing === false. Swallow exactly the first Enter after a
// compositionend so confirming a conversion never jumps.
let swallowNextEnter = false;

document.addEventListener("keydown", (e) => {
  if (e.isComposing || e.keyCode === 229) return; // IME owns these entirely
  if (swallowNextEnter) {
    swallowNextEnter = false;
    if (e.key === "Enter") {
      e.preventDefault();
      return;
    }
  }
  const handled = dispatch({ type: "key", key: e.key, ctrl: e.ctrlKey, composing: false });
  if (handled) e.preventDefault();
});

bar.addEventListener("compositionend", (e) => {
  swallowNextEnter = true;
  const t = e.target as HTMLInputElement;
  if (t.id === "q") dispatch({ type: "query", value: t.value });
});

bar.addEventListener("input", (e) => {
  if ((e as InputEvent).isComposing) return; // wait for compositionend
  const t = e.target as HTMLInputElement;
  if (t.id === "q") dispatch({ type: "query", value: t.value });
});

list.addEventListener("click", async (e) => {
  const target = e.target as HTMLElement;
  if (target.closest("#gear")) {
    await invoke("open_settings").catch((err) => showToast(String(err)));
    return;
  }
  const sess = target.closest<HTMLElement>(".sess");
  if (!sess) return;
  jump({
    tmuxSession: sess.dataset.session!,
    windowIndex: Number(sess.dataset.window),
    paneIndex: Number(sess.dataset.pane),
  } as SessionCard);
});

// Summon always resets to normal mode / first card of the UNFILTERED list.
listen("panel-shown", () => dispatch({ type: "shown" }, filterCards("", cards)));

listen<SessionCard[]>("sessions-updated", (e) => {
  const prevIndex = visible.findIndex((s) => s.card.paneId === ui.selectedPaneId);
  cards = e.payload;
  const nextVisible = filterCards(ui.query, cards);
  ui = reconcile(ui, nextVisible, prevIndex < 0 ? 0 : prevIndex);
  render();
});

// Persist window position (debounced) so the panel reopens where it was left.
let moveTimer: ReturnType<typeof setTimeout> | undefined;
getCurrentWindow().onMoved(({ payload }) => {
  clearTimeout(moveTimer);
  moveTimer = setTimeout(() => {
    invoke("save_window_pos", { x: payload.x, y: payload.y }).catch(() => {});
  }, 500);
});

render();
