# regret

**Your git history already knows which commits caused problems. regret surfaces them.**

*Built for a world where your swarm of AI agents collaborates on GitHub repos with mine.*

---

## Why This Exists

Every codebase accumulates mistakes: commits that introduce bugs, regressions that slip through review, changes that get reverted or patched shortly after landing. The evidence is sitting right there in your git history—reverts, follow-up fixes, churn patterns—but surfacing it manually is tedious and error-prone.

`regret` automates this. It mines your git history for high-confidence signals that a commit caused trouble, ranks the culprits, and shows you the evidence. No configuration, no external services, no training data. Just run `regret` and see what your history reveals.

### What regret finds

| Signal | What it means |
|--------|--------------|
| **Reverts** | A commit was explicitly undone—clear evidence of regret |
| **Linked fixes** | A follow-up commit explicitly references the culprit it's fixing |
| **Patch-id matches** | Same logical change applied twice (with `--deep`) |

### Who this is for

- **Solo developers** who want to spot patterns in their own work
- **Teams** looking for data-driven insight into code quality
- **CI pipelines** that want to flag high-churn commits automatically
- **AI coding agents** that need structured evidence for debugging decisions

---

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
git config commit.template ~/.cmdrvl/state/regret/repos/<repo-id>/commit-template.txt
```

`regret --init` is safe and idempotent. To overwrite existing files, use `--force`.
The exact repo-scoped path is printed by `regret --init` in `ADOPTION.md`.

Disable (local repo):

```bash
git config --unset commit.template
```

Optional advisory hook (local repo only):

```bash
cp ~/.cmdrvl/state/regret/repos/<repo-id>/hooks/commit-msg .git/hooks/commit-msg
chmod +x .git/hooks/commit-msg
```

If you already have a commit-msg hook, merge manually.

Agent instruction snippet (paste into Claude/Codex system prompts; also written by `regret --init` under `~/.cmdrvl/state/regret/repos/<repo-id>/agent-snippets/`):

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

`regret` reads `~/.cmdrvl/config/regret/repos/<repo-id>/config.toml` when present. Legacy repo-local `.regret/config.toml` is copied there on first run and left in place with a deprecation notice. Defaults:
- `ranking.default_since = "30d"`
- `scan.bootstrap_since = "45d"`

Use `--deep` to enable patch-id equivalence (slower, higher recall); default runs include canonical reverts + linked-fix trailers only.
