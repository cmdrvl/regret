# APR Round 2 — Plan Assurance Review

**Date:** 2026-01-27
**Workflow:** default
**Oracle:** Claude Opus 4.5
**Status:** PASS (clarifications applied)

---

## Documents Reviewed

- **Plan:** `PLAN_FOR_ASSURANCE_LAYER.md`
- **Implementation Guidance:** `IMPLEMENTATION_GUIDANCE.md`

---

## Issues Identified

### Issue 1: Patch-ID Computation Ambiguity
**Location:** §8.2 (Manual Revert Equivalence)
**Severity:** Medium
**Resolution:** Added explicit subsection "B) Patch-ID computation (git-compatible)" specifying:
- Use git's native `patch-id --stable` algorithm
- LF normalization for cross-platform determinism
- Store as 20-byte SHA-1 in DB

### Issue 2: Coverage Window Backfill Logic
**Location:** §7.2
**Severity:** Low
**Resolution:** Expanded `--scan` mode documentation with:
- Explicit definition of "backfill"
- Range notation: `[requested_since, current_coverage_since)`
- Concrete example: 30d→90d scenario

### Issue 3: CRLF Handling in Patch-ID
**Location:** §15.2 / Implementation Guidance
**Severity:** Medium
**Resolution:** Added explicit CRLF→LF normalization rule in both documents

### Issue 4: Bootstrap vs First-Run UX
**Location:** §3.2 / §10.1.6
**Severity:** Low
**Resolution:** Differentiated NO_EVENTS activation block by cause:
- `REASON: coverage_incomplete`
- `REASON: no_signals_detected`
- `REASON: signals_outside_window_or_threshold`

### Issue 5: Rate Denominator Clarification
**Location:** §10.1.4
**Severity:** Low
**Resolution:** Added explicit definitions:
- `commits` = commits on selected branch in ranking window
- `events` = signals with evidence time in window and confidence >= threshold

---

## Implementation Guidance Updates

- Added "Patch-ID algorithm (git-compatible)" subsection
- Clarified blake3 vs SHA-1 usage (blake3 for fileset/repo-id, SHA-1 for patch-id)
- Added batched patch-id computation guidance

---

## Verdict

**PASS** — All clarifications have been applied. Documents are now implementation-ready with no ambiguities in the identified areas.

---

## Files Modified

- `PLAN_FOR_ASSURANCE_LAYER.md` (+28 lines)
- `IMPLEMENTATION_GUIDANCE.md` (+19 lines)
