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
type TickPayload = { cards: SessionCard[]; favorites: SessionCard[] };

let cards: SessionCard[] = [];
let favs: SessionCard[] = [];
let visible: ScoredCard[] = [];

function showToast(msg: string): void {
  const t = document.createElement("div");
  t.className = "toast";
  t.textContent = msg;
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 4000);
}

const activeList = (): SessionCard[] => (ui.tab === "all" ? cards : favs);

function jump(card: SessionCard): void {
  // Dead favorites have no pane: resume them in a new tmux window instead.
  // Backend hides the panel after a successful switch/resume.
  if (card.paneId.startsWith("dead:")) {
    if (card.transcriptMissing) showToast("transcript missing — cannot resume");
    else
      invoke("resume_session", { sessionUuid: card.sessionUuid }).catch((err) =>
        showToast(String(err)),
      );
    return;
  }
  invoke("jump_to_session", {
    session: card.tmuxSession,
    windowIndex: card.windowIndex,
    paneIndex: card.paneIndex,
  }).catch((err) => showToast(String(err)));
}

function toggleStar(card: SessionCard): void {
  invoke<boolean>("toggle_favorite", {
    sessionUuid: card.sessionUuid,
    cwd: card.cwd,
    title: card.title,
  })
    .then((starred) => {
      // Optimistic local update so an unstar removes the card immediately;
      // the next 2s tick overwrites both lists with backend truth anyway.
      const live = cards.find((c) => c.paneId === card.paneId);
      if (live) live.starred = starred;
      if (starred) {
        if (!favs.some((c) => c.paneId === card.paneId)) favs.push({ ...card, starred });
      } else {
        favs = favs.filter((c) => c.paneId !== card.paneId);
      }
      const prevIndex = visible.findIndex((s) => s.card.paneId === ui.selectedPaneId);
      ui = reconcile(ui, filterCards(ui.query, activeList()), prevIndex < 0 ? 0 : prevIndex);
      render();
    })
    .catch((err) => showToast(String(err)));
}

function apply(action: Action): void {
  if (action.type === "jump") jump(action.card);
  else if (action.type === "toggleStar") toggleStar(action.card);
  else if (action.type === "hide") invoke("hide_panel").catch(() => {});
}

let renderedMode: UiState["mode"] = "normal";

function render(): void {
  visible = filterCards(ui.query, activeList());
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
  list.innerHTML = renderCards(visible, ui, activeList().length, favs.length);
  // Keep the keyboard selection on-screen; "nearest" is a no-op when it
  // already is, so the 2s refresh doesn't yank a hand-scrolled list around.
  list.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
}

function dispatch(ev: Parameters<typeof reduce>[1], withVisible?: ScoredCard[]): boolean {
  const out = reduce(ui, ev, withVisible ?? visible);
  const changed = out.state !== ui;
  const tabChanged = out.state.tab !== ui.tab;
  ui = out.state;
  // The reducer picked index 0 of the list it was handed (the OLD tab's).
  // Re-select index 0 of the new tab's list with the cleared query.
  if (tabChanged) ui = reconcile({ ...ui, selectedPaneId: null }, filterCards(ui.query, activeList()), 0);
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
  const chip = target.closest<HTMLElement>(".chip");
  if (chip) {
    // Clicking the already-active chip is a no-op (Tab would toggle away).
    if (chip.dataset.tab !== ui.tab) dispatch({ type: "key", key: "Tab", ctrl: false, composing: false });
    return;
  }
  const sess = target.closest<HTMLElement>(".sess");
  if (!sess) return;
  // Rows render in `visible` order, so the DOM index finds the card on either tab.
  const card = visible[Array.from(list.querySelectorAll(".sess")).indexOf(sess)]?.card;
  if (!card) return;
  if (target.closest(".star")) toggleStar(card);
  else jump(card);
});

// Summon always resets to normal mode / first card of the UNFILTERED list.
listen("panel-shown", () => dispatch({ type: "shown" }, filterCards("", cards)));

listen<TickPayload>("sessions-updated", (e) => {
  const prevIndex = visible.findIndex((s) => s.card.paneId === ui.selectedPaneId);
  cards = e.payload.cards;
  favs = e.payload.favorites;
  const nextVisible = filterCards(ui.query, activeList());
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
