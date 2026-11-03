import policy from "../../contracts/diagnostic-redaction.json";

// Shared with Rust: apply to string values before serializing JSON.
const rules = [
  ...policy.secrets.map((rule) => ({ pattern: new RegExp(rule.diagnostic_pattern ?? rule.pattern, `g${rule.flags}`), replacement: "[redacted]", preserveIdentifiers: false })),
  ...policy.diagnostic.map((rule) => ({ pattern: new RegExp(rule.pattern, `g${rule.flags}`), replacement: rule.replacement, preserveIdentifiers: "preserve_identifiers" in rule && rule.preserve_identifiers })),
];
const identifierFields = new Set<string>(policy.identifier_fields);

export function redact(text: string, field?: string): string {
  for (const { pattern, replacement, preserveIdentifiers } of rules) {
    if (preserveIdentifiers && field !== undefined && identifierFields.has(field)) continue;
    text = text.replace(pattern, replacement);
  }
  return text;
}

const secretField = new RegExp(policy.secret_field.pattern, policy.secret_field.flags);

/** The containing field supplies credential context even for opaque values. */
export function isSecretField(name: string): boolean {
  return secretField.test(name);
}
