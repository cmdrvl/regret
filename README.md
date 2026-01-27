# regret

`regret` is a single-verb, local-first, deterministic CLI that mines **high-precision regret signals** from git history and reports the top culprits with evidence.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/cmdrvl/regret/main/scripts/install.sh | bash
```

## Quickstart

```bash
regret
```

If you see a coverage warning, expand local coverage deterministically:

```bash
regret --scan --since 180d
```

## Getting the most out of regret (agent-friendly)

Linked-fix trailers unlock the highest-signal, lowest-noise regret events in v0.1: a follow-up fix can explicitly point back to the culprit commit with **forensic certainty**.

Enable the commit template that nudges humans and agents into writing these trailers:

```bash
regret --init
git config commit.template .regret/commit-template.txt
```

Disable (local repo):

```bash
git config --unset commit.template
```

Agent instruction snippet (paste into Claude/Codex system prompts; also written by `regret --init` to `.regret/agent-snippets/regret-linked-fix.md`):

```markdown
# regret: linked-fix trailers (agent rule)

When you make a follow-up fix for a previous commit, add a trailer referencing the culprit commit:

- Add: `Fixes-Commit: <full 40-hex SHA>` in the commit message trailers/footer section.
- Use the full SHA (no prefixes).
- The SHA MUST be the culprit (the change being fixed), not the evidence/fix commit.
```

Minimal example:

1) Culprit commit (already on the selected branch):
- `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa` (example SHA)

2) Follow-up fix commit message (evidence commit):
```text
Fix login regression

Fixes-Commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
```

3) Verify the linked-fix shows up as evidence (robot-friendly):
```bash
regret --ndjson --since 30d
```

Example NDJSON evidence record (fields may include additional additive keys over time, but `type=linked_fix` and `confidence_reason=explicit_trailer` are the core facts):
```json
{"type":"evidence","signal":"linked_fix","culprit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","evidence_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","confidence_reason":"explicit_trailer"}
```
