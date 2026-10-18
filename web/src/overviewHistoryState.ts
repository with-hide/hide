import { create } from "zustand";
import type { SessionSearchCursor, SessionSearchHit, SessionSearchStatus, SessionWorkSearch } from "./snapshot";

export const HISTORY_PAGE_LIMIT = 200;
export type HistoryState = {
  scope: string;
  query: string;
  extra: boolean;
  agent: string;
  status: SessionSearchStatus | "all";
  days: number;
  since: number | null;
  cursors: (SessionSearchCursor | null)[];
  page: number;
  scroll: number;
  selected: string | null;
  requestId: string | null;
  results: SessionSearchHit[];
  next: SessionSearchCursor | null;
  loading: boolean;
  failure: string | null;
  revision: number;
};

export const historyInitial = (scope: string): HistoryState => ({
  scope, query: "", extra: false, agent: "", status: "all", days: 0, since: null,
  cursors: [null], page: 0, scroll: 0, selected: null, requestId: null,
  results: [], next: null, loading: true, failure: null, revision: 0,
});

/** A mailbox is shared with the command palette. Only this exact request may replace History. */
export function historyAnswer(state: HistoryState, answer: SessionWorkSearch | null): HistoryState {
  if (!state.requestId || answer?.options?.request_id !== state.requestId) return state;
  if (answer.loading) return state;
  return { ...state, loading: false, failure: answer.failure, results: answer.results.slice(0, 50), next: answer.next_cursor ?? null };
}

/** One page of rows and a bounded cursor trail, never an accumulating result cache. */
export function historyPage(state: HistoryState, direction: -1 | 1): HistoryState {
  const page = state.page + direction;
  if (page < 0 || page >= HISTORY_PAGE_LIMIT || (direction === 1 && !state.next) || state.loading) return state;
  const cursors = direction === 1 ? [...state.cursors.slice(0, page), state.next] : state.cursors;
  return { ...state, page, cursors, scroll: 0, selected: null, results: [], next: null, requestId: null, loading: true, failure: null, revision: state.revision + 1 };
}

type HistoryStore = { entry: HistoryState; update: (change: (state: HistoryState) => HistoryState) => void };
export const useHistoryStore = create<HistoryStore>((set) => ({
  entry: historyInitial(""),
  update: (change) => set((state) => ({ entry: change(state.entry) })),
}));
