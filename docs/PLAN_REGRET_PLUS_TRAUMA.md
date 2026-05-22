# Plan: Regret + Trauma Guard Integration

> **Status**: Draft proposal
> **Last updated**: 2026-02-02

## Overview

**Regret** mines git history for evidence of problematic commits (reverts, linked fixes).
**Trauma Guard** (from cass-memory) blocks dangerous commands before they execute.

These systems are complementary:
- Regret **discovers** patterns of regret after the fact
- Trauma Guard **prevents** recurrence in real-time

This document proposes tighter integration to create a feedback loop where historical regret informs future prevention.

---

## Current State

### Regret Capabilities
- Detects canonical reverts (`This reverts commit <sha>.`)
- Detects linked-fix trailers (`Fixes-Commit: <sha>`)
- Detects patch-ID equivalence (with `--deep`)
- Outputs NDJSON for robot consumption
- Supports context trailers: `Bead-Ref:`, `Work-Ref:`, `Session-Ref:`

### Trauma Guard Capabilities
- Blocks commands matching dangerous regex patterns
- Dual-scope storage (global + project)
- Severity levels: CRITICAL, FATAL
- Healing mechanism for temporary bypass
- Scans cass sessions for "apology + destruction" patterns

### Gap
Currently there's no automated bridge. Users must:
1. Run `regret` to find problematic commits
2. Manually inspect what made them problematic
3. Manually add patterns to `cm trauma add`

---

## Integration Opportunities

### 1. Trauma Context Trailer (Low effort)

Add a new trailer type that links commits to known traumas:

```
Trauma-Ref: <trauma-id>
```

**Use case**: When a fix commit addresses a trauma-inducing pattern, reference the trauma ID for traceability.

**Regret changes**:
- Parse `Trauma-Ref:` trailers (read-only, context only)
- Include in NDJSON output as `trauma_refs: string[]`
- No scoring impact (like `Bead-Ref:`)

**Implementation**: ~20 lines in `signals.rs`, update NDJSON schema.

---

### 2. Export Culprits for Trauma Consideration (Medium effort)

New flag: `regret --suggest-traumas`

Analyzes high-regret commits and extracts potential dangerous patterns:

```bash
regret --suggest-traumas --min-score 30
```

Output (NDJSON):
```json
{"type":"trauma_suggestion","culprit_sha":"abc123","pattern":"DELETE FROM .* WHERE 1=1","source":"diff_analysis","confidence":0.7}
{"type":"trauma_suggestion","culprit_sha":"def456","pattern":"rm -rf \\$\\{.*\\}","source":"diff_analysis","confidence":0.8}
```

**Logic**:
1. Filter to culprits above score threshold
2. For each culprit, fetch the diff
3. Extract command-like patterns from added lines
4. Match against known dangerous categories (filesystem, database, git, infra)
5. Output suggestions with confidence

**Regret changes**:
- New module: `trauma_extraction.rs`
- Pattern matchers for common dangerous command categories
- New output mode in `output.rs`

**cass-memory changes**: None required (can pipe to `cm trauma add --file -`)

---

### 3. Trauma-Aware Ranking Boost (Medium effort)

If a culprit commit contains patterns that match existing traumas, boost its visibility.

**Use case**: Highlight commits that touched "known dangerous territory" even if they weren't reverted yet.

```bash
regret --trauma-aware --trauma-file ~/.cass-memory/traumas.jsonl
```

**Logic**:
1. Load trauma patterns from specified file(s)
2. For each culprit, check if diff matches any active trauma pattern
3. Add `trauma_match: boolean` and `matched_traumas: string[]` to output
4. Optionally boost score: `score = base_score * (1 + 0.2 * trauma_matches)`

**Regret changes**:
- New module: `trauma_loader.rs` (parse JSONL format)
- Integration in ranking logic
- New fields in NDJSON output

---

### 4. Shared Pattern Library (Higher effort)

Create a common format for dangerous patterns that both tools consume:

```yaml
# ~/.cmdrvl/config/regret/repos/<repo-id>/patterns.yaml or .cass/patterns.yaml
version: 1
patterns:
  - id: fs-recursive-delete
    regex: "rm\\s+-rf\\s+/"
    category: filesystem
    severity: fatal
    description: "Recursive delete from root"

  - id: db-unbounded-delete
    regex: "DELETE\\s+FROM\\s+\\w+\\s*($|;|WHERE\\s+1)"
    category: database
    severity: critical
    description: "DELETE without meaningful WHERE clause"
```

**Benefits**:
- Single source of truth for both tools
- Can be committed to repos for team sharing
- Version-controlled pattern evolution

**Changes**:
- Regret: New loader for patterns.yaml, use in `--suggest-traumas` and `--trauma-aware`
- cass-memory: New loader alongside existing traumas.jsonl

---

### 5. Bidirectional Sync Command (Higher effort)

New command: `regret trauma-sync`

```bash
# Pull traumas from cass-memory, use for trauma-aware ranking
regret trauma-sync --pull

# Push high-confidence suggestions to cass-memory
regret trauma-sync --push --min-confidence 0.8
```

**Implementation considerations**:
- Requires cass-memory to be installed (optional dependency)
- Could shell out to `cm` or read/write files directly
- Need deduplication logic (don't add patterns that already exist)

---

## Proposed Phasing

### Phase 1: Documentation + Trailers (v0.2)
- [ ] Add `Trauma-Ref:` trailer parsing (context only)
- [ ] Document manual workflow in README
- [ ] Add example under `~/.cmdrvl/state/regret/repos/<repo-id>/agent-snippets/`

### Phase 2: Export Suggestions (v0.3)
- [ ] Implement `--suggest-traumas` flag
- [ ] Pattern extraction from diffs
- [ ] NDJSON output for suggestions

### Phase 3: Trauma-Aware Ranking (v0.4)
- [ ] `--trauma-file` flag to load external patterns
- [ ] `--trauma-aware` mode with ranking boost
- [ ] Trauma match fields in NDJSON output

### Phase 4: Shared Patterns + Sync (v0.5+)
- [ ] Shared pattern format specification
- [ ] `regret trauma-sync` command
- [ ] Integration tests with cass-memory

---

## Usage Examples

### Manual Workflow (Today)

```bash
# 1. Find high-regret commits
regret --ndjson --min-confidence 0.9 | jq '.ranked_culprits[:3]'

# 2. Inspect a culprit
regret sha:abc1234
git show abc1234

# 3. Extract the dangerous pattern manually
# (e.g., you see "DELETE FROM users" without WHERE)

# 4. Add to trauma guard
cm trauma add "DELETE FROM users($| WHERE 1)" --severity critical

# 5. Verify it's active
cm trauma list
```

### With Phase 2 (Suggested)

```bash
# 1. Get trauma suggestions automatically
regret --suggest-traumas --min-score 30 > suggestions.jsonl

# 2. Review and filter
cat suggestions.jsonl | jq 'select(.confidence > 0.8)'

# 3. Import to cass-memory (proposed cm feature)
cat suggestions.jsonl | cm trauma import --stdin --review
```

### With Phase 3 (Trauma-Aware)

```bash
# Rank with trauma awareness - highlights commits touching known danger zones
regret --trauma-aware --trauma-file ~/.cass-memory/traumas.jsonl

# CI: Fail if any high-score commit also matches a known trauma
regret --ndjson --trauma-aware --fail-if "any_trauma_match and max_score > 20"
```

---

## Open Questions

1. **Pattern format**: Should we standardize on regex, or support glob/literal modes too?

2. **Confidence calibration**: How do we assign confidence to extracted patterns? Based on:
   - Number of times the pattern appeared in reverted commits?
   - Time-to-revert (faster revert = higher confidence)?
   - Category (filesystem > logging)?

3. **Deduplication**: When suggesting traumas, how do we avoid duplicates with existing traumas?
   - Exact regex match?
   - Semantic similarity?
   - Let the user decide?

4. **Scope inference**: Should `--suggest-traumas` infer global vs project scope?
   - Patterns from many repos → global
   - Patterns from one repo → project

5. **Opt-in vs opt-out**: Should trauma-awareness be:
   - Explicit flag (`--trauma-aware`)
   - Default if trauma file exists
   - Config option in `~/.cmdrvl/config/regret/repos/<repo-id>/config.toml`

---

## Non-Goals

- **Real-time blocking**: Regret is post-mortem analysis, not a runtime guard. That's Trauma Guard's job.
- **ML/heuristics**: Both tools prioritize deterministic, mechanically-explainable confidence.
- **Network dependencies**: Both tools are local-first.

---

## References

- [cass-memory README](https://github.com/Dicklesworthstone/cass_memory_system) - Trauma Guard documentation
- [regret PLAN_FOR_ASSURANCE_LAYER.md](./PLAN_FOR_ASSURANCE_LAYER.md) - Core regret specification
- Project Hot Stove - Internal codename for Trauma Guard safety system
