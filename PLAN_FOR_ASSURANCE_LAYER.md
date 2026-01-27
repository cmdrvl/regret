# PLAN_FOR_ASSURANCE_LAYER.md

> **Project Codename:** `regret` (assurance layer)
> **Primary Innovation:** evidence-first, local-first outcome signals mined from git history (optional CI later)
> **Architecture Pattern:** Rust + SQLite(WAL) cache + single-verb CLI + robot mode (NDJSON)
> **Repo:** `cmdrvl/regret`
> **Cargo Package:** `cmdrvl-regret`
> **Binary:** `regret`
> **Plan Version:** 2026-01-27.18

---

## 0. Background (Why This Exists)

Swarm development (humans + AI agents shipping in parallel) is rapidly commoditizing “execution infrastructure” (orchestration, coordination, guards, connectors). What doesn’t commoditize quickly is **longitudinal outcome feedback**: a deterministic way to measure “what shipped but rapidly needed correction,” then feed that back into planning and verification.

`regret` is that outcome-feedback loop. It is explicitly:
- **Evidence-first:** every score is backed by concrete evidence commits.
- **Deterministic:** stable schemas, stable ordering, stable exit codes.
- **Local-first:** no cloud services, no network required for core use.
- **Fast-by-default:** default runs stay fast even as history grows.

---

## 1. Definitions (No Vibes)

**Regret event**: a correcting action that is strong evidence a previously delivered change required rapid correction.

**Culprit**: the delivered change being corrected. v0.1 culprits are **commit-scoped** only: `sha:<sha>`.

**Evidence**: the correcting commit that justifies a signal (e.g., the revert commit).

**Signal**: a typed regret event derived from evidence with:
- `weight` (severity)
- `confidence` in `[0.0, 1.0]` (certainty of attribution)
- `time_to_regret_hours` (evidence_time − culprit_time)

**Ranking window**: the interval used to aggregate signals into scores for output. v0.1 applies the ranking window to **evidence time** (“regret observed in this window”), not culprit time.

**Selected branch (v0.1)**: one branch only. Default is the repo’s detected default branch, overridable by config. v0.1 does not scan multiple branches/worktrees.

**Scan coverage**: how far back in history the cache is known to be complete for the selected branch. Exposed so neither humans nor agents mistake partial data for truth.

**Hotspot (v0.1)**: a surface (file path) with repeated regret events from multiple distinct culprits within the ranking window (thresholded). If no hotspot exists, v0.1 prints a deterministic **Top surface** fallback (clearly labeled).

**PR number (v0.1, display-only)**: extracted only when trivially reliable:
- commit subject matches `^Merge pull request #(\\d+)`
- used only for display metadata; scoring/grouping stays commit-scoped

---

## 2. Hard Constraints (Non-Negotiable)

### 2.1 Single-Verb CLI

`regret` is a single verb. No subcommands. Behavior is controlled by flags plus an optional positional `sha:<sha>` id.

### 2.2 Determinism Contract

When `--ndjson` is set:
- stdout contains **only** NDJSON (no banners, no logs)
- record ordering is deterministic
- schemas are versioned and stable
- diagnostics go to stderr only, and only with `--debug`

### 2.3 Performance Targets (Measured, Not Vibes)

All targets are p95 on a typical dev machine and tracked by a benchmark harness.

Definitions:
- **Warm run:** cache exists; OS page cache warm (run twice; report second); no new commits on selected branch.
- **Cold run:** cache missing or OS cold (approximated by deleting cache DB).

Targets:
- Default run warm (no new commits, head-check fast path): **< 15ms**
- Default run warm (ranking query only): **< 40ms**
- Incremental scan warm (typical 1–50 new commits): **< 80ms**
- 90-day scan on ~50k-commit repo:
  - cold: **< 10s**
  - warm rerun: **< 3s**
- Peak RSS during 90-day cold scan: **< 150MB**

### 2.4 Local-First, Easy Install, Beads-Compatible (Not Reliant)

- Core v0.1 requires only local git history.
- Beads compatibility is future, opt-in, best-effort enrichment; never required for correctness.
- CI integrations are optional and must not change scoring deterministically.

---

## 3. Core UX (What Users Must Feel)

### 3.1 Default Run (The 90% Path)

`regret` with no args MUST:
1) Check if `HEAD(selected_branch)` changed since last scan using an **O(1) ref-tip check**; if unchanged and cache-valid, skip scan entirely.
2) **Scan only new commits since the last run** (O(new commits); independent of `--since`).
3) Run ranking over the configured ranking window (default `30d`).
4) Print brutally minimal human output:
   - Top 5 culprits
   - One Hotspot line (or Top surface fallback)
   - One Rate line
   - One Coverage line **only if** the window is not fully covered

### 3.2 Day-1 Usefulness Without Lying

First run must be fast *and* not a dead end:
- bootstrap scan horizon: `scan.bootstrap_since` (default `45d`)
- compute high-precision signals in that horizon
- if zero rows in the default window, print a deterministic “activation” block with exact next commands (no generic advice)

---

## 4. CLI Specification (Single Verb, No Subcommands)

### 4.1 Invocation Grammar

```
regret [sha:<sha>] [FLAGS]
```

### 4.2 Mode Precedence (No Ambiguity)

1) `--init`: install templates/snippets; ignore id and ranking flags
2) `--doctor`: read-only diagnostics; ignore id and ranking flags
3) `--scan`: scan-only; ignore id; do not print rankings
4) `sha:<sha>` present: explain mode for culprit
5) default: incremental scan + ranking

### 4.3 Flags (v0.1 Surface Area Budget)

Core:
- `--version` (print version and exit)
- `--init`
- `--scan` (scan-only)
- `--all` (scan: full rebuild)
- `--doctor`
- `--deep` (doctor only; enables slow checks)
- `--no-scan` (default mode only; skip scan step)
  - uses cached data only; does not check for new commits
  - fails with exit code 1 if `cache_valid=false` or `coverage_valid=false`
  - useful for CI jobs that only need ranking from a previously-scanned cache

Windowing:
- `--since <duration|date>` (ranking window; scan backfill window in `--scan` mode)
- `--until <date>` (ranking end; scan backfill end in `--scan` mode)

Ranking:
- `--limit <n>` (default: 5)
- `--min-confidence <0.0-1.0>` (default: 0.0)

Output:
- `--table` (default)
- `--ndjson`
- `--debug` (stderr only; never contaminates NDJSON)

Gating:
- `--fail-if "<expr>"` (exit code 3 on violation)

Future:
- `--json` (single object) and TOON output (explicitly not v0.1)

### 4.4 Time Parsing (Unambiguous)

Accepted:
- Duration: `Nh`, `Nd`, `Nw` (e.g. `24h`, `30d`, `2w`)
- Date: `YYYY-MM-DD` (as `00:00:00Z`)
- RFC3339: `YYYY-MM-DDTHH:MM:SSZ`

Defaults:
- ranking: `--since 30d`, `--until now` (with a single frozen `until` captured once per run; see §7.4)
- `--scan`: `--since` required unless `--all`

### 4.5 `--fail-if` (Deterministic, v0.1-Useful)

Grammar:
```
<expr> := <term> (("and" | "or") <term>)*
<term> := <metric> <op> <number>
<op>   := ">" | ">=" | "<" | "<=" | "==" | "!="
```

Metrics (v0.1):
- `regret_events`
- `max_score`
- `events_per_100_commits`

Rules:
- short-circuit evaluation
- metrics computed lazily
- no percentile gates in v0.1 (sparse signals make percentiles unstable)

Example:
```bash
regret --since 7d --min-confidence 0.9 --fail-if "regret_events >= 1 and max_score > 20"
```

### 4.6 Exit Codes (Deterministic)

| Code | Meaning | When |
|------|---------|------|
| 0 | Success | Normal completion, no policy violations |
| 1 | Runtime error | Invalid cache, can't open repo, scan lock held, permission denied, unresolvable SHA |
| 2 | Usage error | Invalid flags, malformed arguments, unknown options |
| 3 | Policy violation | `--fail-if` expression evaluated true |

Rules:
- exit codes are stable and part of the contract
- `--ndjson` mode uses the same exit codes; errors go to stderr
- `--doctor` exits 0 even if it reports warnings (diagnostics are informational)

### 4.7 Configuration File (Optional)

Location: `<repo_root>/.regret/config.toml`

Format: TOML (chosen for simplicity and Rust ecosystem alignment)

Schema (v0.1):
```toml
# .regret/config.toml

[scan]
bootstrap_since = "45d"        # default bootstrap horizon for first run

[ranking]
default_since = "30d"          # default --since for ranking window
weights.revert = 10            # weight for revert signals
weights.linked_fix = 8         # weight for linked_fix signals

[hotspot]
min_events = 2                 # minimum events to qualify as hotspot
min_culprits = 2               # minimum distinct culprits
min_confidence = 0.0           # minimum signal confidence

[cache]
wal_checkpoint_threshold_mb = 64  # trigger PASSIVE checkpoint when WAL exceeds this
```

Rules:
- all settings have sensible defaults; config file is optional
- CLI flags override config file values where applicable
- unknown keys are ignored with a warning (forward compatibility)
- invalid values fail with exit code 2

---

## 5. Security Model (Pragmatic, Explicit)

Threat model:
- reads local git repository objects (commits, messages, diffs)
- writes local cache under repo root
- no network in core

Non-negotiable safety:
- **never persist commit bodies** in v0.1 (subjects only)
- validate/sanitize all file paths from git
- parameterized SQL only
- safe cache path creation (no symlink components; best-effort `O_NOFOLLOW`)
- scan locking to prevent concurrent writers

### 5.1 Cache Path Safety (Symlinks + TOCTOU)

On any write to `<repo_root>/.regret/` (`--init`, scan lock, DB create/migrate):
- walk each path component with `lstat`; refuse if any component is a symlink
- create files with mode `0700`/`0600`
- use `O_NOFOLLOW` where supported (best-effort elsewhere)

Locking:
- lock file: `<repo_root>/.regret/scan.lock`
- take exclusive advisory lock (`flock` / `LockFileEx`) before any DB write
- readers do not lock (WAL supports concurrent reads)

### 5.2 Commit Message Safety

- store `commit.subject` only (default truncate 80 chars for human output)
- commit bodies may be read during ingestion to extract structured facts, but MUST be stream-parsed and discarded: **commit bodies are never persisted; only derived facts are stored**
- parse needed structured facts during ingestion:
  - canonical revert line
  - `Fixes-Commit:` / `Fixes-SHA:` trailers
- do not persist body; do not echo body in output

### 5.3 File Path Validation (Fast + Safe)

Rules:
- reject `..` segments
- reject absolute paths
- reject NUL bytes
- normalize separators to `/`
- truncate >4096 bytes with a marker

Performance requirement:
- implement validation as a byte-scan with early exits (no `PathBuf`, no Unicode normalization)
- during scans, keep an in-memory LRU mapping `validated_path_bytes -> file_id` to avoid repeated DB round-trips

---

## 6. Data Model & Cache (SQLite WAL, Minimal, Fast)

### 6.1 Cache Location and Identity (Privacy-Preserving)

Repo root: `git rev-parse --show-toplevel`

Cache root: `<repo_root>/.regret/` (single location in v0.1)

Stable repo identity for NDJSON:
- `git_common_dir = git rev-parse --git-common-dir`
- `repo_id = blake3(canonical_path(git_common_dir))`
- do not include origin URL in `repo_id`

### 6.2 SQLite Settings (Required)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -64000;      -- 64MB
PRAGMA mmap_size = 268435456;    -- 256MB
```

Checkpoint policy:
- never checkpoint on every incremental scan
- checkpoint only on:
  - `--scan --all` completion (TRUNCATE)
  - WAL exceeds `wal_checkpoint_threshold_mb` (default: 64MB; PASSIVE checkpoint)

### 6.3 Minimal Schema (v0.1)

Authoritative schema lives in `docs/schema/sqlite/v1.sql`. Plan sketch:

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE file (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL CHECK(length(path) <= 4096)
);

CREATE TABLE commit (
  sha TEXT PRIMARY KEY CHECK(length(sha) == 40),
  time_utc TEXT NOT NULL,
  subject TEXT,
  pr_number INTEGER,
  pr_source TEXT,      -- 'merge_commit' or NULL
  patch_id BLOB,       -- NULL unless computed (manual revert matching)
  patch_id_rev BLOB,   -- NULL unless computed
  files_hash BLOB      -- NULL unless needed for surfaces
);

CREATE TABLE fileset (
  files_hash BLOB PRIMARY KEY,
  file_count_total INTEGER NOT NULL,      -- before ignore filtering
  file_count_included INTEGER NOT NULL,   -- after ignore filtering
  blob BLOB NOT NULL                      -- varint-encoded sorted INCLUDED file ids
);

CREATE TABLE signal (
  id INTEGER PRIMARY KEY,
  ref TEXT NOT NULL,                 -- selected branch name
  type TEXT NOT NULL CHECK(type IN ('revert','linked_fix')),
  time_utc TEXT NOT NULL,            -- evidence time
  culprit_sha TEXT NOT NULL,
  evidence_sha TEXT NOT NULL,
  weight INTEGER NOT NULL,
  confidence REAL NOT NULL,
  time_to_regret_hours REAL NOT NULL,
  culprit_files_hash BLOB,           -- may be NULL until needed
  evidence_files_hash BLOB            -- may be NULL until needed
);

CREATE INDEX signal_time ON signal(time_utc);
CREATE INDEX signal_culprit ON signal(culprit_sha);
```

Schema/migrations:
- `meta.schema_version` integer
- migrations are sequential and idempotent
- higher-than-known schema_version: refuse with clear error

---

## 7. Git Ingestion & Scanning (Performance-Critical)

### 7.1 Selected Branch Resolution (v0.1)

Default branch detection:
- prefer `refs/remotes/origin/HEAD` if present
- else fall back to `main` then `master` then current `HEAD` branch
- store resolved `selected_branch` in config on first run (so behavior is stable)

### 7.2 Scanning Rules (Fast by Default)

Default mode scanning:
- **always incremental** based on DAG delta since `last_scanned_head`
- independent of ranking `--since`
- if `HEAD` unchanged: scan step is skipped

`--scan` mode:
- incremental scan by default
- if `--since/--until` provided: backfill scan coverage window
  - "backfill" means: extend `coverage_since_utc` backward to the requested `--since` boundary
  - only scan commits in the newly-extended range `[requested_since, current_coverage_since)`
  - do NOT rescan commits already within the existing coverage window
  - example: if coverage is 30d and user runs `--scan --since 90d`, scan only the [90d-ago, 30d-ago) range
- `--all`: rebuild cache from scratch

Coverage:
- store `coverage_since_utc` per selected branch in `meta`
- expose coverage in NDJSON and in human output only when incomplete for the requested window

Rewrite detection:
- if cached `last_scanned_head` is not reachable from current branch head, treat as rewrite:
  - warn in human output and doctor
  - mark cache as `coverage_valid=false` (hard state) to prevent silent partial truth
  - recommend `regret --scan --all`

Shallow clones:
- detect shallow repositories (`.git/shallow` or libgit2 equivalent)
- treat scan coverage as bounded by the shallow boundary; surface this explicitly in `--doctor` and NDJSON meta/stats
- never pretend full coverage is possible without fetching history

### 7.3 Plumbing/Backend Choice (Measured)

Ingestion MUST NOT spawn a subprocess per commit.

Allowed approaches:
- batched git plumbing (preferred for portability)
- libgit2 (if faster in benchmarks)

v0.1 rule: pick the fastest measured default; keep the other as fallback only if it is meaningfully different (avoid permanent double-maintenance without payoff).

Commit message handling (hard rule, no options):
- ingestion may read commit bodies to parse canonical revert lines and trailers, but MUST stream-parse and discard them; **commit bodies are never persisted**

### 7.4 O(1) Fast Path + Deterministic Time (Engineering Depth, No New Options)

Fast path goal: “nothing changed” must be cheap and correct under packed refs, shallow clones, and detached HEAD scenarios.

Persist (in `meta`, per selected branch/ref):
- `ref_name` (the resolved reference that defines the selected branch, e.g. `refs/heads/main` or `refs/remotes/origin/main`)
- `last_scanned_ref_oid` (OID of the selected ref tip when the scan finished)
- `last_scanned_graph_tip` (OID of the commit-graph tip used for incremental scan; same as ref tip in v0.1 but kept explicit)
- `cache_valid` (boolean): invalidated on schema migration failures, rewrite detection, or any state where we cannot trust incremental correctness
- `coverage_since_utc` and `coverage_since_oid` (see §7.2)
- `coverage_valid` (boolean): set false on rewrite detection until a full rebuild (`--scan --all`) completes

Fast path check (O(1), no subprocess):
- use libgit2 (or equivalent internal git object access) to resolve `ref_name` → current `ref_oid`
- if `cache_valid=true` AND `coverage_valid=true` AND `ref_oid == last_scanned_ref_oid`, skip scan entirely

DAG delta scan (O(new commits)):
- compute new commits by walking from `ref_oid` while hiding `last_scanned_graph_tip` (or equivalent revwalk range)
- this guarantees work proportional to new commits, independent of repository history size

Deterministic `until` (frozen once):
- in all modes, `until` is computed exactly once and propagated to: ranking, rate, coverage completeness check, output rendering, and `--fail-if`
- default behavior:
  - if user provided `--until`, use it
  - else if `GITHUB_ACTIONS=true`, set `until` to the selected branch tip’s committer time (stable per commit; CI-friendly)
  - else set `until` to wall-clock `now()` captured once at program start
- `--ndjson` MUST include:
  - `meta.window_until_utc`
  - `meta.window_until_source` in `{flag, github_actions_head_committer_time, wall_clock}`

---

## 8. Signals (v0.1: High Precision, Non-Heuristic)

v0.1 ships only these signal types:

Implementation invariant (applies to all signals):
- evidence and culprit commit bodies may be read to parse revert lines/trailers, but MUST be stream-parsed and discarded; **commit bodies are never persisted; only derived facts are stored**

### 8.1 `revert` (Explicit Canonical Line)

Detect evidence commit bodies containing canonical revert line:
- `This reverts commit <40-hex-sha>.`

Emit:
- `type = revert`
- `culprit_sha = <sha>`
- `evidence_sha = <revert_commit_sha>`
- `confidence = 0.95`
- `confidence_reason = canonical_revert_line`

### 8.2 `revert` (Manual Revert Equivalence via Patch-ID)

Purpose: deterministic recall boost without heuristic overlap.

Algorithm (exact; deterministic; bounded; cached; no heuristics):

A) Candidate evidence commits
- Evidence candidates are commits within the scan coverage horizon whose **SUBJECT contains `revert` or `rollback`** (ASCII case-insensitive byte scan).
- No other candidate sources. No message-body heuristics.

B) Patch-ID computation (git-compatible)
- Use git's native `patch-id --stable` algorithm for portability and consistency with existing tooling.
- Normalization rules (applied before hashing):
  - Strip all whitespace-only changes
  - Normalize line endings to LF (critical for cross-platform determinism)
  - Ignore diff context line counts
  - Hash only the `+`/`-` content lines with file paths
- The resulting patch-id is a 40-hex SHA-1 (git's format) stored as 20 bytes in the DB.

C) Matching rule (first parent or empty tree)
- For an evidence candidate commit `E`: compute `patch_id(E)` from `diff(E, first_parent(E))` (or against the empty tree if `E` is a root commit).
- For a culprit candidate commit `C`: compute `patch_id_rev(C)` from the reverse of `diff(C, first_parent(C))` (or reverse diff against the empty tree if `C` is a root commit).
- A match occurs iff `patch_id(E) == patch_id_rev(C)`.

D) Search bounds (same horizon; time-bounded)
- Only consider culprit candidates `C` where:
  - `C.time_utc <= E.time_utc`, and
  - `C.time_utc >= coverage_since_utc` (same scan coverage horizon).
- Use a time bucket index to avoid O(N²) comparisons:
  - bucket culprits by hour (or day) within the horizon
  - search buckets from `E.time_utc` backward to `coverage_since_utc`
  - in each bucket, compute `patch_id_rev(C)` lazily for commits whose `patch_id_rev` is missing, then compare to `patch_id(E)`

E) Collision resolution
- If multiple culprits match the same evidence `E`, choose the culprit with the maximal `C.time_utc` (closest prior). Tie-break by lexical `C.sha`.
- If one culprit matches multiple evidence commits, emit one signal per evidence commit (distinct `evidence_sha`).

F) Caching
- Store `patch_id` and `patch_id_rev` per SHA (`commit.patch_id`, `commit.patch_id_rev`).
- Compute lazily only when needed; never precompute across all commits.
- Never recompute once stored.

G) Confidence
- Manual patch-id revert emits:
  - `confidence = 0.90`
  - `confidence_reason = patch_id_equivalence`

### 8.3 `linked_fix` (Explicit Trailers)

Detect evidence commits with trailers (parsed from commit body, not persisted):
- `Fixes-Commit: <sha>`
- `Fixes-SHA: <sha>` (alias)

Rules:
- accept 7–40 hex prefixes; resolve against local repo deterministically
- if ambiguous prefix: do not emit a signal; record a debug diagnostic only with `--debug`

Emit:
- `type = linked_fix`
- `culprit_sha = <resolved_sha>`
- `evidence_sha = <fix_commit_sha>`
- `confidence = 0.92`
- `confidence_reason = explicit_trailer`

---

## 9. Ranking, Surfaces, and Output Semantics

### 9.1 Scoring

Score per culprit in ranking window:
- `score = Σ(weight)` over signals whose **evidence time** is in `[until-since, until]` and whose `confidence >= --min-confidence`

Weights (v0.1 defaults, configurable):
- `revert`: 10
- `linked_fix`: 8

### 9.2 Surfaces (Hotspots + Top Surface Fallback)

Surfaces are file paths derived from file sets. v0.1 keeps this cheap:
- compute/store file sets only for commits that are culprits or evidence in emitted signals
- do not precompute file sets for all commits in the window

Ignore patterns (default-on, configurable via `[surfaces]` in config):
- `**/node_modules/**`
- `**/vendor/**`
- `**/target/**` (Rust)
- `**/dist/**`, `**/build/**`
- `**/*.lock`, `**/package-lock.json`, `**/yarn.lock`, `**/pnpm-lock.yaml`
- `**/*.min.js`, `**/*.min.css`
- `**/*.generated.*`, `**/*_generated.*`
- `**/.git/**`

Config override (in `.regret/config.toml`):
```toml
[surfaces]
ignore_patterns = ["**/node_modules/**", "**/vendor/**"]  # replaces defaults
additional_ignore = ["**/my_generated/**"]                 # extends defaults
```

Ignored-file count is reported only in explain/NDJSON diagnostics (not in default human output)

Hotspot:
- requires:
  - `hotspot.min_events >= 2` (default: 2; config-file only in v0.1)
  - `hotspot.min_culprits >= 2` (default: 2; config-file only in v0.1)
  - `signal.confidence >= hotspot.min_confidence` (default: 0.0; config-file only in v0.1)
- hotspot thresholds are config-file settings, not CLI flags (keeps CLI surface minimal)
- if no hotspot: print deterministic **Top surface** (facts only; no labels):
  - the single surface with maximal regret score in the window (ties broken deterministically by path)

### 9.3 Evidence-Time Windowing (Make Surprise Visible)

Because the window is evidence-time, humans can see old culprits in a recent report. v0.1 human output MUST include:
- `culprit_date` and/or `culprit_age`

Explain output MUST include:
- `culprit_time_utc`
- `evidence_time_utc`
- `time_to_regret_hours`

---

## 10. Outputs (Brutally Minimal Human, Rich Robot)

### 10.1 Human Output (Default `--table`)

Default human output MUST be brutally minimal and 100% evidence-only: no interpretations, no “likely”, no labels.

Default human output contains ONLY these blocks in this order:

1) Header line (single line; facts only):
- `regret <tool_version> repo=<repo_basename> branch=<selected_branch> scan=<skipped|ran> new_commits=<n> scanned_commits=<n> coverage_days=<n>`

2) TOP table (max 5 rows) with columns exactly:
- `id` (value: `sha:<short_sha>`)
- `score`
- `events`
- `ttr_p50_h`
- `culprit_date_utc`
- `culprit_age`
- `subject_trunc`
- `conf` (value: minimum confidence among included events for that culprit in the window)

3) One line: HOTSPOT (facts only) OR TOP_SURFACE fallback (facts only):
- `HOTSPOT path=<path> score=<n> events=<n> culprits=<n>`
- `TOP_SURFACE path=<path> score=<n> events=<n> culprits=<n>` (when hotspot thresholds are not met)

4) One line: RATE (facts only; include denominators):
- `RATE events_per_100_commits=<x> events=<e> commits=<c>`
- `commits` = count of commits on the selected branch within the ranking window `[until-since, until]`
- `events` = count of regret events (signals) with evidence time in the ranking window and confidence >= `--min-confidence`

5) Coverage line ONLY when incomplete OR `coverage_valid=false`:
- `COVERAGE status=<complete|incomplete|invalid> coverage_since_utc=<ts> coverage_valid=<0|1>`
- if `status=incomplete`, append exact next command:
  - `NEXT: regret --scan --since <window_since>`
- if `status=invalid`, append exact next command:
  - `NEXT: regret --scan --all`

6) If zero events: activation block (facts + exact next commands only):
- `NO_EVENTS reverts_detected_in_coverage_horizon=<n> linked_fix_trailers_detected_in_coverage_horizon=<n> coverage_days=<n>`
- Differentiate cause based on coverage vs signal presence:
  - If `coverage_days < ranking_window_days`: coverage is incomplete
    - `REASON: coverage_incomplete`
    - `NEXT: regret --scan --since <ranking_window>`
  - Else if `reverts_detected_in_coverage_horizon == 0 AND linked_fix_trailers_detected_in_coverage_horizon == 0`: no signals exist
    - `REASON: no_signals_detected`
    - `NEXT: regret --init`
    - `NEXT: git config commit.template .regret/commit-template.txt`
  - Else: signals exist but none meet confidence threshold or fall in ranking window
    - `REASON: signals_outside_window_or_threshold`
    - `NEXT: regret --since <longer_window> --min-confidence <lower>`

### 10.2 Robot Output (`--ndjson`)

NDJSON record types (v0.1):
- `meta` — exactly one; run metadata
- `rank` — one per culprit in output
- `evidence` — one per signal
- `stat` — aggregate statistics
- `diag` — diagnostics (only with `--debug`, still valid NDJSON)

Required `meta` fields (v0.1):
- `schema_version` (integer, currently `1`)
- `tool_version` (semver string)
- `repo_id` (blake3 hash of git_common_dir; see §6.1)
- `repo_basename` (last path component of repo root)
- `selected_branch` (resolved branch name)
- `window_since_utc`, `window_until_utc` (ISO-8601)
- `window_until_source` (`flag` | `github_actions_head_committer_time` | `wall_clock`)
- `coverage_since_utc` (ISO-8601)
- `coverage_valid` (boolean)
- `cache_valid` (boolean)

Required `stat` records (v0.1):
- `stat.regret_events` — count of signals in ranking window
- `stat.max_score` — highest culprit score
- `stat.events_per_100_commits` — rate metric
- `stat.commits_in_window` — denominator for rate
- `stat.coverage_days` — days of scan coverage

Ordering:
1) `meta` (exactly one)
2) `stat` records (fixed ordering by name)
3) `rank` records (sorted by score desc, then culprit sha)
4) `evidence` records (grouped by culprit sha, then evidence time, then sha)

Contract:
- schema is stable and versioned
- new fields are additive only (no meaning changes without bump)

Confidence must be mechanically explainable:
- `evidence.confidence_reason` is an enum (deterministic, not interpretive):
  - `canonical_revert_line`
  - `patch_id_equivalence`
  - `explicit_trailer`

Surfaces must be explainable without flags:
- `rank.surface_coverage = included/total` (numeric in `[0.0, 1.0]`)
- explain mode includes `surface_included` and `surface_ignored` counts for each evidence row when surfaces are present

Schema docs:
- `docs/schema/ndjson/v1.md` (authoritative)

### 10.3 Explain Mode Output (`regret sha:<sha>`)

When a culprit id is provided (`regret sha:<sha>`), output details for that specific culprit.

Behavior:
- if `<sha>` resolves to a commit that is NOT a culprit (no signals reference it): print `NO_SIGNALS culprit=sha:<sha>` and exit 0
- if `<sha>` is ambiguous or unresolvable: print error to stderr and exit 1
- if `<sha>` is a known culprit: print evidence details

Human output (`--table`, default):
```
CULPRIT sha:<short_sha> score=<n> events=<n> ttr_p50_h=<h> culprit_date_utc=<date>
  subject: <full_subject_truncated_120>

EVIDENCE (sorted by evidence_time desc):
  type        evidence_sha   evidence_date_utc  ttr_h   conf   confidence_reason
  revert      sha:<short>    2026-01-15         2.3     0.95   canonical_revert_line
  linked_fix  sha:<short>    2026-01-10         48.1    0.92   explicit_trailer

SURFACES (top 5 by score):
  path                          score  events
  src/auth/login.rs             18     2
  src/api/handlers.rs           10     1
```

NDJSON output (`--ndjson`):
- emits `meta` record (same as default mode)
- emits one `rank` record for the culprit (or none if not a culprit)
- emits all `evidence` records for that culprit (sorted by evidence_time desc, then evidence_sha)
- does NOT emit `stat` records (stats are for aggregate views)

---

## 11. Workflow Embedding (CI Gates, Hooks, Agents)

### 11.1 `--init` (Idempotent Templates)

`regret --init` MUST:
- create `<repo_root>/.regret/` if missing (safe perms, no symlink components)
- create `<repo_root>/.regret/agent-snippets/` if missing
- write templates/snippets (never auto-enable git config; always deterministic):
  - `.regret/commit-template.txt` (commit message template with commented trailer examples; exact content below)
  - `.regret/ADOPTION.md` (exact enable/disable commands; exact content below)
  - `.regret/agent-snippets/regret-linked-fix.md` (agent prompt snippet; exact content below)
  - `.regret/agent-snippets/regret-session-context.md` (agent prompt snippet for CASS join keys; exact content below)
  - `.regret/ci/github-actions-regret.yml` (copy/paste snippet)
  - `.regret/hooks/commit-msg` (advisory; always exit 0)

`.regret/commit-template.txt` (exact content):
```text
# regret commit template
#
# If this commit fixes a previously delivered change, add an explicit trailer
# referencing the *culprit* commit (full 40-hex SHA; no prefixes):
#
# Fixes-Commit: <full_sha>
# Fixes-SHA: <full_sha>
#
# Optional (context only; never affects scoring; enables CASS/beads join):
# Bead-Ref: <id>
# Work-Ref: <token>
# Session-Ref: <session_id>
```

`.regret/ADOPTION.md` (exact content):
```markdown
# regret adoption

Enable commit template (local repo):

    git config commit.template .regret/commit-template.txt

Disable commit template (local repo):

    git config --unset commit.template

Check current setting:

    git config --get commit.template
```

`.regret/agent-snippets/regret-linked-fix.md` (exact content):
```markdown
# regret: linked-fix trailers (agent rule)

When you make a follow-up fix for a previous commit, add a trailer referencing the culprit commit:

- Add: `Fixes-Commit: <full 40-hex SHA>` in the commit message trailers/footer section.
- Use the full SHA (no prefixes).
- The SHA MUST be the culprit (the change being fixed), not the evidence/fix commit.
```

`.regret/agent-snippets/regret-session-context.md` (exact content):
```markdown
# regret: session context trailers (agent rule)

Always include work tracking trailers in your commits to enable conversation-to-code traceability:

- Add: `Bead-Ref: <bead_id>` if you are working on a tracked issue (e.g., `Bead-Ref: beads-42`)
- Add: `Work-Ref: <token>` if you have a work token from your orchestrator
- Optionally add: `Session-Ref: <session_id>` if your session ID is available

These trailers enable `cass` and other tools to join commit history back to the conversation that produced it. They do not affect regret scoring—they are context only.
```

`--init` output MUST print:
- what files were written (basenames only)
- a deterministic next-steps block with exact commands (no prompts, no interpretation):
```text
Next steps:
  git config commit.template .regret/commit-template.txt
  git config --unset commit.template
```

### 11.2 CI Gates (Warn → Fail)

CI runs should use `--ndjson` and `--fail-if`. Recommended patterns are defined in §14 (Rollout Playbook).

### 11.3 `--doctor` (Read-Only Diagnostics)

`regret --doctor` reports (read-only):
- `db_quick_check` (`PRAGMA quick_check`)
- schema version (current vs expected)
- coverage for selected branch vs requested window
- rewrite detection (`last_scanned_head` reachability)
- cache size (db + wal)

`--doctor --deep` adds:
- `PRAGMA integrity_check` (slow; never default)

---

## 12. Distribution & Install (Jeff-Grade)

Goal: one-line install, cross-platform, verifiable, reversible, and easy to pin.

### 12.1 Supported Install Channels (v0.1+)

Primary (recommended):
- GitHub Releases prebuilt artifacts + checksums + provenance
- one-liner installers:
  - `scripts/install.sh`
  - `scripts/install.ps1`

Secondary (best-effort):
- `cargo binstall cmdrvl-regret`
- `cargo install cmdrvl-regret --locked` (slow; requires Rust toolchain)

Future (nice-to-have, not required for v0.1 correctness):
- Homebrew tap, Scoop manifest

### 12.2 Release Artifact Naming (Deterministic)

Each GitHub Release `vX.Y.Z` publishes:
- `regret-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- `regret-vX.Y.Z-x86_64-apple-darwin.tar.gz`
- `regret-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
- `regret-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
- `regret-vX.Y.Z-x86_64-pc-windows-msvc.zip`
- `SHA256SUMS`
- `SHA256SUMS.sig` (signature; see §12.4)
- `provenance.intoto.jsonl` (build provenance attestation)
- `sbom.cdx.json` (CycloneDX; minimal SBOM)

Archive contents (all platforms):
- `regret` / `regret.exe`
- `LICENSE`
- `README.md` (short)

### 12.3 Install Scripts (One-Line, Pinned, Rollback)

Repository scripts (source of truth):
- `scripts/install.sh`
- `scripts/install.ps1`

One-line usage:
```bash
curl -fsSL https://raw.githubusercontent.com/cmdrvl/regret/main/scripts/install.sh | bash
```
```powershell
irm https://raw.githubusercontent.com/cmdrvl/regret/main/scripts/install.ps1 | iex
```

Pin a version (required for CI reproducibility):
- Unix:
  - `REGRET_VERSION=vX.Y.Z curl -fsSL .../install.sh | bash`
- PowerShell:
  - `$env:REGRET_VERSION="vX.Y.Z"; irm .../install.ps1 | iex`

Pin the installer script itself (supply-chain hardening; recommended for CI):
- Unix:
  - `curl -fsSL https://raw.githubusercontent.com/cmdrvl/regret/vX.Y.Z/scripts/install.sh | bash`
- PowerShell:
  - `irm https://raw.githubusercontent.com/cmdrvl/regret/vX.Y.Z/scripts/install.ps1 | iex`

Defaults:
- install dir:
  - Unix: `${XDG_BIN_HOME:-$HOME/.local/bin}`
  - Windows: `$HOME\AppData\Local\regret\bin`
- script refuses to install if it cannot verify checksums (unless `REGRET_NO_VERIFY=1`)

Installer dependency policy:
- Unix script must run on POSIX sh-compatible shells and only require:
  - `curl` or `wget`
  - `tar` + `gzip` (or `bsdtar`)
  - `shasum` or `sha256sum`
- PowerShell script must run on PowerShell 5+ and use `Invoke-WebRequest` + `Expand-Archive` + built-in hashing.

Post-install self-test (no new installer options; always runs):
- run `regret --version` and fail if non-zero
- run `regret --doctor` inside a temporary empty repo (`git init`) and fail only if the binary cannot execute (doctor may report “cache missing” and still exit 0)
- macOS: if the installed binary has quarantine xattr, print deterministic remediation:
  - `xattr -d com.apple.quarantine "<installed_path>"`
- Windows: print PATH hint if install dir is not on PATH (do not auto-modify PATH)

Rollback strategy:
- installer writes versioned binaries alongside:
  - Unix: `regret@vX.Y.Z` and `regret` symlink (or copy if symlink not allowed)
  - Windows: `regret@vX.Y.Z.exe` and `regret.exe`
- rollback is deterministic:
  - Unix: `ln -sf regret@vX.Y.Z regret`
  - Windows: copy `regret@vX.Y.Z.exe` over `regret.exe`

Manual install (no scripts):
1) Download the correct release asset and `SHA256SUMS`.
2) Verify the SHA256 line for your asset matches.
3) Extract `regret`/`regret.exe` into a directory on PATH.

### 12.4 Checksums + Signature Strategy (Verifiable by Default)

Required verification:
- installer downloads `SHA256SUMS` and verifies the selected artifact hash matches

Signature strategy (best-effort v0.1, hardened over time):
- sign `SHA256SUMS` and publish `SHA256SUMS.sig`
- verify signature in installer when verification tooling is present:
  - preferred: Sigstore `cosign` keyless verification (GitHub OIDC identity pinned to this repo)
  - fallback: skip signature verification but still enforce SHA256SUMS

Policy:
- checksums are mandatory
- signatures are strongly recommended; CI release pipeline must produce them even if installer treats verification as optional

---

## 13. Onboarding & It-Just-Works Guide

### 13.1 60-Second Quickstart (Happy Path)

1) Install:
   - `curl -fsSL .../scripts/install.sh | bash`
2) In any git repo:
   - `regret`
3) If you see `Coverage: incomplete ...`:
   - run the exact suggested command, e.g. `regret --scan --since 180d`
4) To enable linked-fix signals:
   - `regret --init`
   - `git config commit.template .regret/commit-template.txt`

### 13.2 Expected Output Shapes (Deterministic)

Default (`--table`) shape:
- Top 5 table (fixed columns; stable ordering)
- Hotspot/Top surface line
- Rate line
- Optional Coverage line
- Optional Activation block on zero events

Explain (`regret sha:<sha>`) shape:
- human table listing evidence events for that culprit (type, evidence time, ttr, evidence sha, subject)
- `--ndjson` provides full structured evidence records

### 13.3 Troubleshooting (Common Failures → Exact Fix)

Common messages (examples; exact wording is a contract):
- `error: not a git repository`
  - fix: run inside a repo; or set `--debug` to see resolution
- `error: .regret/ contains a symlink component; refusing to write`
  - fix: remove symlink; ensure `.regret/` is a real directory under repo root
- `error: another scan is in progress (scan.lock is held)`
  - fix: wait; or if stale, remove lock only after verifying no `regret` is running
- `error: permission denied writing .regret/`
  - fix: ensure repo is writable; ensure `.regret/` is user-owned; avoid running in read-only workspaces
- `warning: branch history appears rewritten; cache may be stale`
  - fix: `regret --scan --all`
- `warning: window not fully covered`
  - fix: `regret --scan --since <window>`
- `error: cannot execute binary` (macOS quarantine / Gatekeeper)
  - fix: remove quarantine xattr for the installed binary, then retry:
    - `xattr -d com.apple.quarantine "$(command -v regret)"`
- `error: SmartScreen / antivirus blocked regret.exe` (Windows)
  - fix: use GitHub Release checksums/provenance; install to a user-writable directory; re-download if quarantined

Doctor:
- `regret --doctor`
- `regret --doctor --deep` (slow; offline)

---

## 14. Rollout Playbook (Warn → Fail, Safe Defaults)

Goal: teams adopt without breaking workflows or creating noisy gates.

### 14.1 Phase 0 — Local, Low Friction

- Developers/agents run `regret` after merging or after swarm sessions.
- Use `--init` to install templates and CI snippet (but do not auto-enable anything).
- No CI gating yet.

### 14.2 Phase 1 — Informational CI (Non-Blocking)

Add a job that runs:
- `regret --scan --since 180d --until <head_time>`
- `regret --ndjson --since 30d --until <head_time>`
- parse into a job summary (Top 5 + Rate + Coverage completeness)

Semantics:
- informational only (never fails the PR)
- artifacts uploaded (NDJSON + logs)
- no secrets

### 14.3 Phase 2 — Soft Gates (Continue-on-Error)

Run `--fail-if` but do not block merges:
- `continue-on-error: true`
- post failure details in job summary

Recommended initial gates (v0.1 signals are high precision but may be sparse):
- `regret_events >= 1 and max_score > 25` over `--since 7d --min-confidence 0.9`

### 14.4 Phase 3 — Hard Gates (Blocking)

Only after the repo has real coverage and teams are used to the tooling:
- make the same gates blocking
- ratchet thresholds based on historical data (not gut feel)

### 14.5 Pre-Commit/Commit-Msg Hooks (Advisory Only)

`--init` installs `.regret/hooks/commit-msg`:
- never blocks commits (exit 0)
- prints a one-line hint when message contains `Fixes` but no `Fixes-Commit` trailer

---

## 15. E2E & Regression Harness (Determinism, Safety, Cross-Platform)

This harness exists to prevent regressions in:
- determinism (ordering, time handling, schema)
- correctness (signals, scanning, coverage, rewrite detection)
- performance (fast path stays fast)
- safety (cache path, locking, secrets)

Principles:
- fast-by-default PR pipeline
- least-privilege GitHub permissions
- artifacts-first debugging

### 15.1 GitHub Actions Workflow Inventory (Explicit)

Required workflows:

1) `.github/workflows/ci.yml` (fast default)
   - triggers: `pull_request`, `push` to default branch
   - jobs: `fmt`, `clippy`, `build`, `test-nextest`, `lint-scripts`
   - tests via `cargo nextest` with JUnit XML uploaded:
     - artifact: `artifacts/nextest/junit.xml`
   - script linting (fast, cross-platform):
     - `scripts/install.sh`: `shellcheck`
     - `scripts/install.ps1`: `PSScriptAnalyzer`
   - optional UBS scan on changed `*.rs` (non-blocking, step summary)
   - permissions: `contents: read`

2) `.github/workflows/e2e.yml` (targeted end-to-end)
   - triggers: `pull_request` with `paths`:
     - `src/**`, `tests/**`, `migrations/**`, `docs/schema/**`, `.github/workflows/**`, `scripts/**`
   - matrix: `ubuntu-latest`, `macos-latest`, `windows-latest`
   - on failure upload: `artifacts/e2e/**` (goldens, NDJSON, stderr)
   - permissions: `contents: read`

3) `.github/workflows/coverage.yml` (opt-in on PR, gate on main)
   - tool: `cargo llvm-cov`
   - artifacts:
     - `artifacts/coverage/lcov.info`
     - `artifacts/coverage/coverage.json`
   - thresholds (start low; ratchet):
     - line ≥ 60%
     - branch ≥ 50%
   - triggers:
     - `push` to default branch (blocking)
     - `workflow_dispatch`
     - `pull_request` only with label `coverage` (informational)
   - permissions: `contents: read`

4) `.github/workflows/bench.yml` (perf regression; opt-in + nightly)
   - tool: Criterion + macrobench script
   - baseline caching: `actions/cache` keyed by `runner.os + rustc + Cargo.lock`
   - warn: slowdown ≥ 10%
   - fail: slowdown ≥ 20% on default-branch/nightly runs
   - artifacts: `artifacts/bench/**`
   - permissions: `contents: read`

5) `.github/workflows/regret_summary.yml` (PR scan summary; no secrets)
   - trigger: `pull_request`
   - determinism rule: `--until` must be derived from HEAD committer time (never wall-clock)
   - artifacts: `artifacts/regret/ndjson.txt`, `artifacts/regret/stderr.txt`
   - permissions: `contents: read`

6) `.github/workflows/nightly.yml` (deep suite)
   - trigger: nightly schedule + `workflow_dispatch`
   - runs: e2e matrix + coverage + bench
   - permissions: `contents: read`

### 15.2 Golden Output Strategy (No Flake)

Files:
- `tests/fixtures/` (fixture generator code; not static repos committed to git)
- `tests/e2e_init_snapshots.rs` (golden init file contents + init stdout)
- `tests/e2e_table_snapshots.rs`
- `tests/e2e_ndjson_snapshots.rs`
- `tests/snapshots/**`

Rules:
- tests always pass explicit `--since` and `--until`
- tests force:
  - `TZ=UTC`
  - `LC_ALL=C`
  - `LANG=C`
  - `NO_COLOR=1`
  - `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL=<empty-file>`
- normalize paths:
  - replace repo root with `<REPO>`
  - replace cache path with `<CACHE>`
- normalize meta:
  - `meta.repo_id` → `<REPO_ID>`
  - `meta.tool_version` → `<VERSION>`
- enforce deterministic ordering at the source (don’t “sort in tests” to hide nondeterminism)

Fixture git rules:
- fixtures must set `git config core.autocrlf false` and use explicit author/committer dates per commit

Fixture generator (hermetic):
- tests programmatically create a temp repo, then create commits with:
  - fixed author/committer identities
  - fixed `GIT_AUTHOR_DATE` / `GIT_COMMITTER_DATE`
  - explicit message bodies containing revert lines/trailers when needed
- this prevents “fixture drift” and makes failures locally reproducible

Required fixture scenarios (v0.1):
- canonical revert line → emits `revert` with `confidence_reason=canonical_revert_line`
- manual revert patch-id match → emits `revert` with `confidence_reason=patch_id_equivalence`
- linked-fix with unique short SHA prefix → emits `linked_fix` with `confidence_reason=explicit_trailer`
- ambiguous short SHA prefix → MUST NOT emit any signal (deterministic no-op)
- rewritten history: last_scanned_head not reachable → sets `coverage_valid=false` and forces coverage-invalid output
- Windows CRLF edge: ensure patch-id and fileset logic remains stable with CRLF working tree settings
- init acceptance: `regret --init` creates `.regret/commit-template.txt`, `.regret/ADOPTION.md`, and `.regret/agent-snippets/regret-linked-fix.md` with exact contents (snapshot)

### 15.3 Schema Versioning Policy + CI Enforcement

Contracts:
1) NDJSON schema: `docs/schema/ndjson/v1.md`
2) SQLite schema: `docs/schema/sqlite/v1.sql`

Rules:
- additive-only fields without schema bump are allowed only if:
  - existing fields keep meaning
  - ordering rules unchanged
  - readers can ignore unknown fields safely
- rename/removal/meaning change requires schema bump

Enforcement:
- `schema_lock` tests:
  - assert schema version constants match docs
  - assert required fields exist per record type
  - fail if schema changes without docs update

SQLite schema invariant tests (migrations-hardening; no new CLI):
- CI test creates a v1 DB using `docs/schema/sqlite/v1.sql`, then runs the current binary in `--doctor` mode against it
- asserts:
  - schema_version is readable and matches
  - required indexes exist
  - `PRAGMA foreign_key_check` is empty

Deterministic ordering property tests:
- insert signals out-of-order into the DB and assert `--ndjson` ordering remains invariant (the binary must enforce ordering in queries, not rely on insertion order)

### 15.4 Install Script E2E (Release-Grade)

Add a small E2E test that:
- builds artifacts in CI
- runs `scripts/install.sh` / `scripts/install.ps1` against a local “release assets” directory (no network)
- verifies:
  - checksum verification works
  - rollback mechanism works
  - `regret --version` matches expected

This runs:
- in `release.yml` always
- optionally in nightly

---

## 16. Release Automation (Fast, Verifiable, Cross-Platform)

### 16.1 Workflows and Triggers

Primary workflow: `.github/workflows/release.yml`

Triggers:
- `workflow_dispatch`
- `push` to default branch when `Cargo.toml` version changes (crate `cmdrvl-regret`)

Policy:
- derive `version` from `Cargo.toml`
- if tag `v<version>` does not exist:
  - create tag
  - build artifacts
  - publish GitHub Release

Permissions:
- release job only: `contents: write`
- all other jobs: `contents: read`

### 16.2 Build Matrix (v0.1 Minimum)

Required targets:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Build rules:
- pinned Rust toolchain via `rust-toolchain.toml`
- `cargo build --release --locked`
- strip symbols where appropriate
- archive naming per §12.2

### 16.3 Provenance + SBOM (Minimal but Real)

Release pipeline produces:
- `provenance.intoto.jsonl` (SLSA-style provenance, GitHub OIDC)
- `sbom.cdx.json` generated by `cargo cyclonedx` (or equivalent)

### 16.4 Reproducible Builds (Practical)

v0.1 goal: “reproducible-by-policy”:
- pinned toolchain + locked deps
- no build timestamps in output formatting
- nightly job optionally rebuilds Linux x86_64 and compares SHA256 to last release artifact (signal-only at first, then gate once stable)

---

## 17. Milestones (Deliverables + Real DoD)

### M0 — Repo Skeleton + Safety + Fast CI (Day 1)

Deliver:
- CLI parses flags + id; mode precedence correct
- cache root: `<repo_root>/.regret/` (single cache)
- scan lock + symlink-component checks
- NDJSON stdout cleanliness
- `--doctor` (quick checks only)
- `.github/workflows/ci.yml` with nextest + junit artifact
- docs stubs: `README.md` quickstart, `docs/schema/**` placeholders

DoD:
- CI green on Linux/macOS/Windows (fmt/clippy/build/test)
- running `regret --ndjson` produces valid NDJSON only

### M1 — Scanning + Coverage + Fast Path (Days 2–3)

Deliver:
- selected branch detection; stable config
- bootstrap scan horizon (`45d`)
- incremental scan by DAG delta (independent of `--since`)
- rewrite detection + clear remediation
- coverage tracking (`coverage_since_utc`)

DoD:
- warm no-new-commits fast path < 15ms on fixtures
- coverage line appears only when incomplete

### M2 — Signals v0.1 + Golden Outputs (Days 4–6)

Deliver:
- `revert` canonical line
- patch-id manual revert equivalence (bounded, cached)
- `linked_fix` trailers (Fixes-Commit / Fixes-SHA)
- minimal human output + activation block
- NDJSON schema v1 + sqlite schema v1 docs
- snapshot tests for `--table` and `--ndjson`

DoD:
- golden output tests pass on Linux/macOS/Windows deterministically
- schema_lock tests enforced in CI

### M3 — Surfaces + Hotspot/Top Surface + CI Summary (Days 7–9)

Deliver:
- lazy file sets for culprits/evidence only
- ignore patterns
- hotspot + top surface fallback line
- `.github/workflows/regret_summary.yml` (PR job summary; no secrets; deterministic `--until`)

DoD:
- default runs remain fast on fixtures as signals increase

### M4 — Distribution & Onboarding (Days 10–12)

Deliver:
- `scripts/install.sh` + `scripts/install.ps1` (checksum verification, pinning, rollback)
- `.github/workflows/e2e.yml` (targeted matrix, artifact uploads on failure)
- `--init` templates/snippets + docs “10-second adoption loop”
- `--doctor --deep` implemented

DoD:
- install scripts validated in CI against local artifacts (no network)
- onboarding quickstart works from clean checkout

### M5 — Release Automation + Coverage/Bench (Days 13–16)

Deliver:
- `.github/workflows/release.yml` builds and publishes artifacts (matrix)
- checksums + signature artifact + provenance + sbom attached
- `.github/workflows/coverage.yml` (thresholds; fast-by-default triggers)
- `.github/workflows/bench.yml` (baseline caching; warn/fail; nightly)
- `.github/workflows/nightly.yml` deep suite

DoD:
- tagged release produces correct artifacts for all targets
- install scripts succeed against GitHub Release assets
- CI stays fast-by-default; heavy jobs opt-in/scheduled

---

## 18. Risks (Explicit, Ranked)

### 18.1 Trust Risks (False Positives)

- Heuristic fix-forward detection is deferred (future) specifically to avoid trust-killing false positives.
- PR inference is display-only and only when trivially reliable.

### 18.2 Operational Risks (Cache Corruption / Concurrency)

- scan.lock prevents concurrent writers
- WAL + limited checkpoints prevent IO thrash
- migrations are explicit; schema mismatches fail loudly

### 18.3 Install/Release Risks

- wrong-arch downloads: mitigated by target-triple mapping + smoke tests
- antivirus/quarantine: mitigate with checksums, provenance, and eventual platform signing/notarization (future)

---

## 19. Future Ideas (Explicitly Out of v0.1)

### 19.1 Fix-Forward Inference (Heuristic, Trust-Risky)

File-set overlap and rework inference remain future until validated with low-noise evaluation.

### 19.2 PR/Queue Inference Beyond Merge Commits

Anything beyond canonical merge commit subjects is future, opt-in, and must be provably reliable.

### 19.3 Multi-Branch + Worktree Support

Future: shared cache across worktrees and multi-branch scanning (requires careful identity + locking design).

### 19.4 Additional Outputs

- `--json` (single object) next
- TOON output last

### 19.5 Optional Enrichment Sources

- beads mapping (best-effort)
- mcp-agent-mail or other local sources (optional evidence/context only; never required; never affects scoring by default)

### 19.5.1 CASS Compatibility (Conversation Join Keys)

Enable joining regret signals to conversation history via `cass` (conversation/session search):

**Join keys (parsed from commit trailers, never affect scoring):**
- `Work-Ref: <token>` / `Bead-Ref: <id>` — preferred join key; links commit to tracked work item
- `Session-Ref: <session_id>` — optional stronger join; links commit to specific agent session

**Behavior:**
- If present, regret extracts these trailers and includes them in NDJSON `evidence` records as additive context fields:
  - `work_ref` (string or null)
  - `bead_ref` (string or null)
  - `session_ref` (string or null)
- These fields are **display/join only**; they never affect `weight`, `confidence`, or `score`
- Aligns with "enrichment must not change scoring" rule

**Usage:**
- External tools (cass, beads) can join on these fields to surface conversation context for a regret signal
- Example: `cass search --session <session_ref>` to find the agent conversation that produced a culprit commit

**Agent snippet (added to `--init`):**
- `.regret/agent-snippets/regret-session-context.md` instructs agents to include `Bead-Ref` and optionally `Session-Ref` in commits

### 19.6 Bot Detection (`is_bot`)

Future: add `is_bot` column to commit table for filtering/weighting bot-authored commits differently.
- detection heuristics: author email patterns, commit message patterns, known bot names
- usage: optional filtering in ranking, separate bot-vs-human rate stats
- not in v0.1 schema to avoid unused columns; add when there's a concrete use case
