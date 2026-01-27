# regret — Implementation Guidance (Rust)

This document is implementation guidance for building `regret` exactly as specified in the PLAN: single-verb, deterministic, evidence-only, blazing-fast, local-first, beads-compatible but not dependent.

## Principles (non-negotiable)

### Deterministic by default
- Freeze `now` once per invocation. Treat it as `until` and pass it everywhere (scan stats, window, rate, top, gating, NDJSON).
- When `--ndjson` or `--json` is active, **stdout must contain only JSON**. Debug logs go to stderr and must be gated behind `--debug`.
- Output ordering must be stable:
  - stable sorting keys (score desc, then evidence time asc/desc as defined, then id lexical)
  - stable tie-breakers (lexical SHA, lexical id)

### Evidence-only
- Output only facts derived from git history and deterministic computations.
- No “likely cause”, no “unstable”, no inferred advice in default output.
- Any “confidence” must be mechanically explainable (e.g., `confidence_reason=canonical_revert_line|patch_id_equivalence|explicit_trailer`).

### Fast path matters more than cold path
- Warm `regret` should be ~O(1) when HEAD unchanged.
- Avoid “work per commit” unless required by signals.
- Lazily compute expensive artifacts only for commits involved in signals (culprit/evidence), not for all commits.

### Local-first + safe cache
- Never require network.
- Cache writes must be safe: refuse symlink components, use atomic writes, lock correctly, prevent path traversal.

---

## Architecture seams (extensibility without user-facing surface)

Keep seams internal (compile-time), not dynamic plugin loading.

### 1) `GitBackend`
Abstract the read operations. Two backends can exist:
- `libgit2` (preferred if fastest / simplest in-process)
- batched git plumbing (fallback)

API shape (example):
- `head_oid() -> Oid`
- `revwalk(range) -> Iterator<CommitMeta>`
- `commit_message(oid) -> &str` (streamed, not persisted)
- `diff_against_parent(oid) -> DiffView` (for patch-id)
- `files_changed(oid) -> Iterator<PathBytes>` (for surfaces)

### 2) `SignalDetector`
Each signal is a module implementing:
- input: read-only view of commits in coverage window + accessors for message parsing, patch-id cache, etc.
- output: deterministic `SignalEvent` records

v0.1 modules:
- `signals::revert_canonical_line`
- `signals::revert_patchid_equivalence`
- `signals::linked_fix_trailers`

### 3) `SurfaceProvider`
Responsible for file set hashing + ignore rules + counts.
- Must be lazily invoked only for commits that appear in signals (culprit/evidence).
- Must return:
  - `files_hash` (blake3 over canonicalized sorted file ids)
  - `file_count_total`
  - `file_count_included`

### 4) `Store` (SQLite)
Keep all DB logic behind a `Store` boundary:
- `begin_scan_run(until, head) -> RunId`
- `upsert_commit_meta(...)`
- `insert_signal(...)`
- `get_top(by, since, until, limit) -> Vec<RankRow>`
- `get_evidence(id) -> Vec<EvidenceRow>`
- `cache_state()` (schema version, last scanned head, coverage_since, coverage_valid)

---

## What to do (best practices)

## Deterministic time & windows
- Capture `until` once.
- Compute `window_start = until - since_duration` deterministically (no per-call now()).
- Store and use UTC internally; format in output consistently (UTC).

## Commit message parsing without persisting bodies
- Read commit messages from git objects during ingestion.
- Extract only derived facts:
  - `revert_target_sha` (canonical line)
  - `linked_fix_target_sha` (Fixes-Commit / Fixes-SHA trailer)
  - `work refs` (Bead-Ref/Work-Ref/etc.)
- Do **not** store full message bodies in SQLite.

Implementation tips:
- Use a fast byte parser for trailers (don’t split by lines into Vec<String>).
- Parse canonical revert line with a byte scan for exact substring pattern and a strict SHA regex.

## Patch-id equivalence (bounded and safe)
Implement the bounded patch-id manual revert logic exactly:
- Evidence candidates: commits in coverage horizon with SUBJECT containing `revert` or `rollback` (ASCII case-insensitive scan).
- For each evidence commit `E`:
  - compute `patch_id(E)` from diff(E, parent(E))
  - search culprits `C` in coverage horizon with `C.time <= E.time`
  - compute `patch_id_rev(C)` from reverse diff(C, parent(C))
  - match iff `patch_id(E) == patch_id_rev(C)`
- Collision resolution:
  - multiple culprits: choose max(C.time), tie-break lexical SHA
  - one culprit matches multiple evidence commits: emit one signal per evidence

### Patch-ID algorithm (git-compatible)
Use git's native `patch-id --stable` algorithm for portability:
- Preferred: shell out to `git patch-id --stable` via batched plumbing
- Alternative: reimplement the algorithm in Rust if perf requires it

Normalization rules (critical for cross-platform determinism):
- **CRLF handling**: normalize all line endings to LF before computing patch-id
  - This matches git's internal behavior
  - Required for Windows compatibility—same commit must produce same patch-id on all platforms
- Strip whitespace-only changes
- Ignore diff context line counts
- Hash only `+`/`-` content lines with file paths

The resulting patch-id is a 40-hex SHA-1 (git's format), stored as 20 bytes in SQLite.

Performance:
- Cache patch ids (`patch_id`, `patch_id_rev`) keyed by SHA in SQLite.
- Do not compute patch ids for all commits—only for candidates involved in matching.
- Batch patch-id computation: collect all candidate SHAs, run one `git patch-id` invocation per batch.

## SQLite performance
- WAL mode.
- Prepared statements.
- Batch inserts inside a transaction per scan.
- Keep indexes minimal but aligned to query patterns.
- Use `quick_check` for default doctor; deep integrity check only when explicitly requested.

Schema invariants:
- `fileset` table must include `file_count_total`, `file_count_included`.
- Store interned file path IDs to reduce DB size; store varint-encoded sorted ID lists as blob.

## Cache safety & atomicity
- Cache dir `.regret/` must refuse symlink components.
- Use atomic writes for config/templates (`write temp -> fsync -> rename`).
- Use a dedicated lock file (`.regret/scan.lock`) with an exclusive lock; do not lock the DB file as the sole mechanism.
- Never follow symlinks when opening/writing files where the OS supports `O_NOFOLLOW`.

## Output discipline (human + robot)
Human default output (minimal, evidence-only):
- header line with scan summary
- top table
- one hotspot or one top-surface line
- one rate line
- coverage line only if incomplete or coverage invalid
- activation block on zero events (deterministic counters + exact next commands)

Robot outputs:
- NDJSON record types must be stable and versioned.
- Add explicit fields for:
  - `confidence_reason`
  - `surface_included`, `surface_total`, `surface_coverage`
  - `culprit_time`, `evidence_time`, `time_to_regret_hours`

## Install + init as adoption lever
`regret --init` should:
- create `.regret/commit-template.txt` containing commented trailers:
  - `Fixes-Commit: <full_sha>`
  - `Fixes-SHA: <full_sha>`
  - optional `Bead-Ref`, `Work-Ref`
- write `.regret/ADOPTION.md` with exact `git config commit.template ...` commands
- write `.regret/agent-snippets/regret-linked-fix.md` for copy/paste into agent prompts
- perform a self-check (paths writable, lock workable) and print next steps deterministically

---

## What NOT to do (common failure modes)

### Don’t add heuristics in v0.1
- No “fix-forward by file overlap” in core ranking for v0.1.
- No probabilistic or ML-based classification.
- No fuzzy PR mapping beyond canonical merge commit formats.

### Don’t store sensitive text
- Don’t persist commit bodies, PR descriptions, environment variables, or raw diffs.
- Don’t print full paths by default in human output.

### Don’t implement dynamic plugins
- No dlopen/WASM plugin system inside the binary.
- Keep extension via internal traits and external NDJSON-consuming tools.

### Don’t make CI/bench flakey
- Bench thresholds should be generous and based on medians; always pin toolchain.
- Snapshot tests must normalize:
  - timestamps (use fixed commit dates in fixtures)
  - paths (use placeholders)
  - repo id (deterministic fixture)

### Don’t let “doctor” be expensive by default
- Avoid integrity checks unless explicitly requested.
- Avoid scanning entire history in doctor.

---

## Testing strategy (make it “battle-tested”)

## Unit tests
- message parsing: canonical revert line, trailer parsing, SHA validation
- path validation: null bytes, `..`, absolute paths, separator normalization

## Fixture-based integration tests (hermetic)
Create a temp git repo programmatically:
- fixed author/committer names and times
- commits that demonstrate:
  - canonical revert line
  - manual revert (patch-id match)
  - linked-fix trailer
  - ambiguous short SHA prefix (must not emit)
  - rewritten history invalidates coverage

Tests assert:
- `regret --ndjson` matches golden snapshots exactly
- output ordering is stable
- cache coverage rules are respected

## E2E tests (cross-platform)
- run `regret --init`, ensure files exist and content matches snapshot
- run `regret` on fixture repo, verify minimal output shape
- ensure Windows line endings don’t break patch-id computation (normalize diffs consistently)

## Performance tests
- microbench:
  - trailer parsing
  - patch-id hashing
  - fileset hashing
  - SQLite batch insert
- macrobench:
  - cold scan 30d/90d on a known repo snapshot
  - warm `regret` with unchanged head

---

## Rust-specific considerations

### Prefer predictable allocations
- Use `bytes`/`&[u8]` parsing for commit messages and paths.
- Avoid `PathBuf` conversions in hot loops; treat paths as bytes and validate minimally.

### Error handling
- Use `thiserror` for typed internal errors.
- Map to a small set of exit codes (usage vs runtime vs policy).
- In `--ndjson` mode, still exit non-zero on failure; never emit partial mixed output.

### Concurrency
- Concurrency should not change output ordering.
- Any parallel processing must gather results and sort deterministically before output.
- Keep concurrency bounded; subprocess concurrency must be controlled.

### Hashing
- Use `blake3` for:
  - repo/cache IDs (avoid origin URLs)
  - fileset hashes (sorted file IDs → blake3)
- Use git's native SHA-1 patch-id for patch-id equivalence (see §Patch-ID algorithm above)
  - Do NOT use blake3 for patch-ids—git compatibility matters for debugging and tooling interop

---

## “Awesome implementation” checklist

- Warm `regret` on unchanged repo: <120ms and no IO spikes.
- Default output is minimal and never interpretive.
- Zero-events output provides deterministic counters and activation steps.
- Patch-id manual revert matching is bounded, cached, and explainable.
- Cache is safe against symlinks/path traversal.
- NDJSON schema is stable and versioned; external tools can build on it.
- E2E harness proves determinism across platforms.

---
