/** Quoting shared by the native desktop caller and commands typed into PowerShell panes. */
export const powershellQuote = (value: string): string => `'${value.replace(/['\u2018-\u201b]/g, "$&$&")}'`;

/** The native Windows argv convention, used by ProcessStartInfo and Start-Process. */
export function windowsArgument(value: string): string {
  return `"${value.replace(/(\\*)"/g, "$1$1\\\"").replace(/(\\+)$/g, "$1$1")}"`;
}

/** Keep PowerShell 5's native argument rewriting out of a fixture program's argv. */
export function powershellCommand(argv: string[]): string {
  const [program, ...args] = argv;
  if (!program) throw new Error("a pane command needs an executable");
  const argumentsOption = args.length ? ` -ArgumentList ${powershellQuote(args.map(windowsArgument).join(" "))}` : "";
  return `Start-Process -FilePath ${powershellQuote(program)}${argumentsOption} -NoNewWindow -Wait`;
}
