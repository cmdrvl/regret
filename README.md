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

By default, `regret` ranks evidence in the last 30 days (configurable) and freezes `--until` once per invocation for deterministic output.

If you see a coverage warning, expand local coverage deterministically:

```bash
regret --scan --since 180d
```

For reproducible runs (CI/snapshots), pin the window explicitly:

```bash
regret --since 30d --until 2024-01-02T00:00:00Z
```

## Getting the most out of regret (agent-friendly)

Linked-fix trailers unlock the highest-signal, lowest-noise regret events in v0.1: a follow-up fix can explicitly point back to the culprit commit with **forensic certainty**.

Enable the commit template that nudges humans and agents into writing these trailers:

```bash
regret --init
git config commit.template .regret/commit-template.txt
```

`regret --init` is safe and idempotent. To overwrite existing files, use `--force`.

Disable (local repo):

```bash
git config --unset commit.template
```

Optional advisory hook (local repo only):

```bash
cp .regret/hooks/commit-msg .git/hooks/commit-msg
chmod +x .git/hooks/commit-msg
```

If you already have a commit-msg hook, merge manually.

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
{"type":"evidence","signal_type":"linked_fix","culprit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","evidence_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","confidence_reason":"explicit_trailer"}
```

NDJSON schema reference: `docs/schema/ndjson/v1.md`

## Determinism (short)

- `--until` is frozen once and used for age/coverage/rate calculations.
- `--since` filters evidence by when regret happened (evidence time), not culprit age.
- `--all` removes the window bound and implies a full-history scan.

## Advanced config (optional)

`regret` reads `.regret/config.toml` when present. Defaults:
- `ranking.default_since = "30d"`
- `scan.bootstrap_since = "45d"`

Use `--deep` to enable patch-id equivalence (slower, higher recall); default runs include canonical reverts + linked-fix trailers only.
