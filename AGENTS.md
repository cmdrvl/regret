# AGENTS.md - regret

`regret` is a single-verb, local-first, deterministic Rust CLI that mines high-precision regret signals from git history and reports the top culprits with evidence.

---

## RULE 0 - THE FUNDAMENTAL OVERRIDE PEROGATIVE

If the user tells you to do something, even if it conflicts with what follows, YOU MUST LISTEN TO THE USER. The user is in charge.

## RULE 1 - ABSOLUTE (DO NOT EVER VIOLATE THIS)

You may NOT delete any file or directory unless the user explicitly gives the exact command in this session.

- This includes files you just created (tests, tmp files, scripts, etc.).
- You do not get to decide that something is "safe" to remove.
- If you think something should be removed, stop and ask. You must receive clear written approval before any deletion command is even proposed.

Treat "never delete files without permission" as a hard invariant.

### IRREVERSIBLE GIT & FILESYSTEM ACTIONS - DO-NOT-EVER BREAK GLASS

Absolutely forbidden unless the user provides the exact command AND explicit approval in the same message:

- `git reset --hard`
- `git clean -fd`
- `rm -rf`
- Any command that can delete or overwrite code/data

Rules:

1. If you are not 100% sure what a command will delete/overwrite, do not propose or run it. Ask first.
2. Prefer safe tools: `git status`, `git diff`, `git stash`, copying to backups, etc.
3. After approval, restate the command verbatim, list what it will affect, and wait for confirmation.
4. When a destructive command is run, record in your response:
   - The exact user text authorizing it
   - The command run
   - When you ran it

If that audit trail is missing, act as if the operation never happened.

## RULE 2 - BEADS/BR DATABASE SAFETY (ABSOLUTE, IF USED)

`regret` is "beads-compatible but not dependent", but if this repo is using Beads (`br`) in a session:

**SQLite + WAL = DATA LOSS RISK.** Improper handling can destroy uncommitted data.

**Note:** `br` (beads_rust) is non-invasive - it has NO daemon and NEVER executes git commands. You must manually run `git add .beads/ && git commit` after `br sync --flush-only`.

### BEFORE Running Parallel Agents That Use `br`

You MUST complete this checklist BEFORE launching any parallel agents/subagents that will run `br update`, `br create`, or any `br` write operations:

```bash
# 1. Check for stale br processes
lsof .beads/beads.db 2>/dev/null | wc -l
# Should be 0 or 1. If more, wait for other agents to finish.

# 2. Run doctor checks
br doctor 2>&1 | rg -n "(FAIL|Error|\\xE2\\x9C\\x96)" || true
# If any failures, STOP. Ask user.

# 3. Verify sync status
br sync --status 2>&1
# Check if DB and JSONL are in sync
```

If ANY check fails: STOP and ask the user. Do NOT proceed.

### FORBIDDEN ACTIONS (WILL DESTROY DATA)

1. NEVER kill processes holding `.beads/beads.db` (the WAL may contain uncommitted transactions).
2. NEVER delete or modify these files manually:
   - `.beads/beads.db`
   - `.beads/beads.db-wal`
   - `.beads/beads.db-shm`
3. NEVER run `rm .beads/beads.db*` to "fix" issues.

When `br sync` fails: STOP IMMEDIATELY and ask the user. Do not attempt process killing or DB/WAL deletion.

---

## Pinned Documents (Source of Truth)

- Master plan: `PLAN_FOR_ASSURANCE_LAYER.md` (plan version `2026-01-27.15`)
- Implementation guidance: `IMPLEMENTATION_GUIDANCE.md`

If code changes would violate the plan's hard constraints (single-verb, determinism contract, evidence-only, local-first), stop and ask first.

---

## Repository Role

**Role**: Outcome feedback ("assurance layer") for swarm development: compute deterministic regret signals from git history.

**What it owns (v0.1)**:
- Single-verb CLI `regret` with `--ndjson` robot mode
- Evidence signals derived from git history (reverts + linked-fix trailers + bounded patch-id equivalence)
- Local cache under repo root (SQLite WAL) with safe/atomic writes
- Deterministic ranking over an evidence-time window with stable ordering

**What it does NOT own (v0.1)**:
- Network dependencies or cloud services
- Probabilistic heuristics / ML classification
- Multi-branch/worktree scanning (selected branch only)
- Persistent storage of raw commit bodies or raw diffs

---

## Key Invariants (must preserve)

- **Single-verb CLI**: no subcommands; mode is only via flags (plus optional `sha:<sha>` id).
- **Deterministic by default**:
  - Freeze `until` once per invocation and pass it through everything.
  - Stable ordering and stable tie-breakers (no "whatever the map yields").
- **Evidence-only output**: no inferred advice in default output; confidence must be mechanically explainable.
- **Robot output discipline**:
  - With `--ndjson` (and future `--json`), stdout contains only JSON.
  - Logs/diagnostics go to stderr and only behind `--debug`.
- **Local-first**: never require network for correctness; never introduce network calls on the hot path.
- **Fast path matters**: warm default run should be ~O(1) when HEAD unchanged and cache-valid.
- **Cache safety**: refuse symlink components, prevent path traversal, use atomic writes + explicit locks.

---

## Development Notes (Rust + SQLite)

### Expected Structure (once scaffold exists)

```
regret/
├── AGENTS.md
├── README.md
├── PLAN_FOR_ASSURANCE_LAYER.md
├── IMPLEMENTATION_GUIDANCE.md
├── Cargo.toml
├── src/
│   └── main.rs
├── crates/               # optional workspace split (core/store/git)
└── tests/
    └── fixtures/         # hermetic git repos created programmatically
```

### Quality Gates

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Guidance:
- Prefer `Result<T, E>` propagation over `unwrap()`/`expect()` in non-test code.
- Do not print nondeterministic data in snapshots; normalize times/paths in fixtures.
- SQLite: keep queries/indexes aligned with planned query patterns; avoid premature schema sprawl.

---

## Testing Guidelines

- Unit tests: commit message parsing (trailers + canonical revert lines), duration/date parsing, stable sorting/tie-breakers, path safety.
- Integration tests: create temp git repos programmatically with fixed author/committer times and known commit graphs; assert `--ndjson` output exactly.
- Performance: avoid per-commit heavy work; compute expensive artifacts only for commits involved in signals (culprit/evidence).

---

## MCP Agent Mail — Coordination for Multi-Agent Workflows

Agent Mail is a mail-like layer that lets coding agents coordinate asynchronously via MCP tools and resources. It provides identities, inbox/outbox, searchable threads, and advisory file reservations, with human-auditable artifacts in Git.

**Same repository:**
1. Register an identity: call `ensure_project`, then `register_agent` using this repo's absolute path as `project_key`.
2. Reserve files before you edit: `file_reservation_paths(project_key, agent_name, ["src/**"], ttl_seconds=3600, exclusive=true)`.
3. Communicate with threads: use `send_message(..., thread_id="FEAT-123")`; check inbox with `fetch_inbox` and acknowledge with `acknowledge_message`.

**Tips:**
- Prefer macros when you want speed: `macro_start_session`, `macro_prepare_thread`, `macro_file_reservation_cycle`.
- Keep reservation patterns minimal and specific; renew rather than holding a broad lock.

---

## Integrating with Beads (Optional, Non-Invasive)

Beads provides a lightweight, dependency-aware issue database and a CLI (`br`) for selecting "ready work," setting priorities, and tracking status.

**Note:** `br` is non-invasive and never executes git commands. After `br sync --flush-only`, you must manually run `git add .beads/ && git commit`.

```bash
br ready              # Show issues ready to work (no blockers)
br show <id>          # Full issue details with dependencies
br update <id> --status=in_progress
br close <id> --reason="Completed"
br sync --flush-only  # Export to JSONL (NO git operations)
```

---

## LANDING THE PLANE (SESSION COMPLETION) - DO NOT STOP EARLY

Work is NOT complete until:

- Quality gates pass (when code changed).
- Changes are committed intentionally (no mystery diffs).
- `git push` succeeds (when this repo has a remote and you're landing work).

### Mandatory Workflow (when you changed code)

1. Quality gates:
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   cargo test
   ```
2. Sync Beads (only if `.beads/` is in use for this repo/session):
   ```bash
   br sync --flush-only
   git add .beads/
   ```
3. Commit + push:
   ```bash
   git status
   git add -A
   git commit -m "..."
   git pull --rebase
   git push
   git status
   ```

### Hard Rules

- NEVER stop at "ready to push" - if you're landing changes, you push.
- If push fails, resolve and retry until it succeeds (or ask the user if there's a policy/permission issue).
- If you see unexpected changes in `git status` that you didn't make, STOP and ask the user how to proceed.

---

## Tool Guidance

### ast-grep vs ripgrep

**Use `ast-grep` when structure matters.** It parses code and matches AST nodes, so results ignore comments/strings, understand syntax, and can safely rewrite code.

**Use `ripgrep` (`rg`) when text is enough.** It's the fastest way to grep literals/regex across files.

Examples (once Rust code exists):

```bash
# Find all uses of unwrap() (avoid in production paths)
ast-grep run -l Rust -p '$EXPR.unwrap()'

# Find all NDJSON writes (stdout discipline)
rg -n 'ndjson|write_ndjson|println!\\(' -S -t rs
```

### cass — Cross-Agent Session Search

`cass` indexes prior agent conversations so we can reuse solved problems.

Rules: never run bare `cass` (TUI). Always use `--robot` or `--json`.

```bash
cass search "regret linked-fix trailer" --robot --limit 5
cass search "sqlite wal lock file" --robot --limit 5
```

---
