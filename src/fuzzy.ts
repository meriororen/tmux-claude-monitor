// fzf-style fuzzy matching, pure and DOM-free.
// Positions are CODE-POINT indices (Array.from), never UTF-16 offsets —
// Japanese titles and emoji must highlight on character boundaries.

import type { SessionCard } from "./render";

export interface FuzzyResult {
  score: number;
  positions: number[];
}

const SEGMENT_SEPARATORS = new Set([" ", "/", "-", "_", ":", "．", "・"]);
// Consecutive runs must dominate segment starts: otherwise a query scattered
// across separators ("d-e-d-u-p") collects a start-bonus per char and beats a
// true run ("dedup").
const CONSECUTIVE_BONUS = 8;
const SEGMENT_START_BONUS = 4;

export function fuzzyMatch(query: string, text: string): FuzzyResult | null {
  const q = Array.from(query.toLowerCase());
  if (q.length === 0) return { score: 0, positions: [] };
  const t = Array.from(text.toLowerCase());

  // Greedy forward subsequence match.
  const positions: number[] = [];
  let ti = 0;
  for (const qc of q) {
    while (ti < t.length && t[ti] !== qc) ti++;
    if (ti >= t.length) return null;
    positions.push(ti);
    ti++;
  }

  let score = 0;
  for (let i = 0; i < positions.length; i++) {
    const p = positions[i];
    score += 1;
    if (i > 0 && positions[i - 1] === p - 1) score += CONSECUTIVE_BONUS;
    if (p === 0 || SEGMENT_SEPARATORS.has(t[p - 1])) score += SEGMENT_START_BONUS;
  }
  return { score, positions };
}

export interface ScoredCard {
  card: SessionCard;
  field: "title" | "session" | "summary" | null;
  positions: number[];
  score: number;
}

export function filterCards(query: string, cards: SessionCard[]): ScoredCard[] {
  if (query === "") {
    return cards.map((card) => ({ card, field: null, positions: [], score: 0 }));
  }
  const scored: ScoredCard[] = [];
  for (const card of cards) {
    const fields: ["title" | "session" | "summary", string][] = [
      ["title", card.title],
      ["session", card.tmuxSession],
      ["summary", card.summary],
    ];
    let best: ScoredCard | null = null;
    for (const [field, text] of fields) {
      const m = fuzzyMatch(query, text);
      if (m && (!best || m.score > best.score)) {
        best = { card, field, positions: m.positions, score: m.score };
      }
    }
    if (best) scored.push(best);
  }
  // Stable sort by score descending (Array.prototype.sort is stable).
  return scored.sort((a, b) => b.score - a.score);
}
