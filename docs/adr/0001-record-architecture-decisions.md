# 0001. Record architecture decisions

- Status: Accepted
- Date: 2026-11-28
- Code: `docs/adr/`, `scripts/check-adr.py`

## Context

`docs/ARCHITECTURE.md` held every architecture decision in one file of about 1.6 MB.
The rule to read it in full before a change was followed rarely, and about half of the pull requests changed it, so pull requests open side by side kept conflicting in it.

## Decision

Architecture decisions are kept as records in Michael Nygard's form, one file per decision under `docs/adr/`, named `NNNN-<slug>.md`: a four-digit number used once and a lowercase slug.

A record starts with its title line, `# NNNN. <Title>`, and three fields:

- `Status`: `Accepted`, `Deprecated`, or `Superseded by NNNN` once a later record reverses it.
- `Date`: the day the decision was first recorded; records 0002 to 0062 carry the day their first section first appeared in `docs/ARCHITECTURE.md`.
- `Code`: the paths the decision owns, each in backticks, a folder ending in `/`; every path exists.

Its body has `## Context`, `## Decision` and `## Consequences`.
Records 0002 to 0062 were moved from `docs/ARCHITECTURE.md` with their text unchanged under `## Decision`; only heading levels and relative link targets changed.
The pull request that next changes one of them sorts its text into the three parts.

A decision that is reversed gets a new record with the next free number, and the old record's Status becomes `Superseded by NNNN` while its text stays as it was.
A change that refines the same decision, or behavior the decision already covers, edits the record in place, in the same pull request as the behavior, as every owning guide in this repository is edited (operator decision, 2026-11-28).
The standard form never edits a record; this repository keeps the current contract in the record, so only a reversal makes a new one.

`docs/ARCHITECTURE.md` is the table of contents: one row per record with its Code paths, then every section heading the file had, kept with its name and anchor and pointing at the record that holds it.
A comment or document that cites `docs/ARCHITECTURE.md, <section>` keeps landing on that heading; a new reference may name the record directly.
A new record adds its row to the table in the same pull request.

A record stays at or under 60 KB, so it can be read whole.

`scripts/check-adr.py` checks every record's name and number, its title line and fields, that its Code paths exist and that it has a `## Decision`, the size limit, and that the table of contents links every record and every anchor it links exists.

## Consequences

Before a change, read the table in `docs/ARCHITECTURE.md` and the records whose Code paths cover the files you change, not every record.
Five records were over 60 KB when they moved: 0004, 0005, 0013, 0050 and 0059.
The check lists them as known until a follow-up splits each, refuses any other record over the limit, and refuses a listed one that has come under it, so the list only shrinks.
Pull requests that change different decisions no longer meet in one file.
