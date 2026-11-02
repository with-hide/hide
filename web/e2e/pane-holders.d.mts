// Types for pane-holders.mjs, which stays plain JavaScript because the web
// shell measurement's shell scripts run it with Node.

export const HOLDER_END_MS: number;
export function ownedHolders(stateDir: string): { file: string; holder: number; program: number | undefined }[];
export function holderStrays(stateDir: string): string[];
export function endPaneHolders(stateDir: string, options?: { endTree?: (pid: number) => void }): void;
