import policy from "../../contracts/diagnostic-redaction.json";

// Shared with Rust: apply to string values before serializing JSON.
const rules = [
  ...policy.secrets.map((rule) => ({ pattern: new RegExp(rule.diagnostic_pattern ?? rule.pattern, `g${rule.flags}`), replacement: "[redacted]" })),
  ...policy.diagnostic.map(({ pattern, flags, replacement }) => ({ pattern: new RegExp(pattern, `g${flags}`), replacement })),
];

export function redact(text: string): string {
  for (const { pattern, replacement } of rules) text = text.replace(pattern, replacement);
  return text;
}

const secretField = new RegExp(policy.secret_field.pattern, policy.secret_field.flags);

/** The containing field supplies credential context even for opaque values. */
export function isSecretField(name: string): boolean {
  return secretField.test(name);
}
