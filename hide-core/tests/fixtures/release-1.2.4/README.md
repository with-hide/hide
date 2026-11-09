# Release 1.2.4 fixtures

Records in the shape the released 1.2.4 daemon writes them, kept so a test proves that a field added since leaves an old record's bytes as they were and that 1.2.4 still reads the new record.

- `project-settings.json`: two Project settings records, one without guardrails (version 1) and one with them (version 2), written by hand from 1.2.4's `ProjectSettings` (`git show v1.2.4:hide-core/src/project_settings.rs`): every field it declares, `None` written as `null` where it has no `skip_serializing_if`.
