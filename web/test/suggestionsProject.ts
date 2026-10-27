import type { ProjectSuggestions, SuggestionRow, SuggestionSignal } from "../src/snapshot";

// A Project's 제안 tab as the core sends it (herdr-core/src/model/suggestions.rs),
// with every field at its quiet value, so a test names only what it is about.

const SIGNALS: SuggestionSignal[] = ["failed_tool", "agent_wait", "correction", "label_missed", "check_failed", "change_request"];

export function suggestionsProject(overrides: Partial<ProjectSuggestions> = {}): ProjectSuggestions {
  return {
    workspace_id: "p",
    loading: false,
    unavailable: null,
    enabled: true,
    intro_seen: true,
    status: "idle",
    ai: "ready",
    github: "connected",
    settings: { suggestions_enabled: true, quantity: "normal", apply_mode: "confirm", signals: SIGNALS.map((signal) => ({ signal, enabled: true, unavailable: null })), dismissed: [] },
    rows: [],
    counts: { suggestions: 0, rail: 0 },
    ...overrides,
  };
}

export function suggestionRow(id: string, overrides: Partial<SuggestionRow> = {}): SuggestionRow {
  return {
    id,
    source: "notebook",
    group: "choose",
    state: "new",
    discomfort: `discomfort ${id}`,
    cause: `cause ${id}`,
    fix: `fix ${id}`,
    outcome: "pr",
    basis: null,
    auto: false,
    hold: null,
    escalated: null,
    returned_after_skip: false,
    evidence: [],
    attempt: null,
    watch: null,
    ...overrides,
  };
}
