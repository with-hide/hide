// Types for sidebar-geometry.mjs, which stays plain JavaScript because the
// private design review tooling, kept outside this checkout, imports it from Node.
import type { Page } from "@playwright/test";

export type RowBox = { x: number; y: number; width: number; height: number };

export function rowBoxes(page: Page): Promise<Record<string, RowBox>>;
export function rowTargets(page: Page): Promise<{ key: string; row: string; control: string | null }[]>;
export function agentColumns(page: Page): Promise<{ list: string; rows: { pane: string; depth: number; mark: number; title: number }[] }[]>;
export function rowPartProblems(page: Page): Promise<string[]>;
export function sidebarColumns(page: Page): Promise<{ times: number[]; chevrons: number[] }>;
export function sidebarOverflow(page: Page, width: string | null): Promise<string[]>;
export function sidebarRowsFit(page: Page): Promise<string[]>;
