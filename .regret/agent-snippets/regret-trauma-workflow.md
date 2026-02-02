# regret + Trauma Guard workflow (agent rule)

When you discover high-regret commits, consider whether the underlying pattern should become a trauma to prevent recurrence.

## Discovery → Prevention Loop

1. **Find high-regret commits**:
   ```bash
   regret --ndjson | jq '.ranked_culprits[] | select(.score > 20)'
   ```

2. **Inspect the culprit**:
   ```bash
   regret sha:<short-sha>
   git show <full-sha>
   ```

3. **If the culprit contains a dangerous pattern** (destructive command, unbounded delete, force push, etc.), extract it and add to Trauma Guard:
   ```bash
   cm trauma add "<regex-pattern>" --severity critical --message "Discovered via regret sha:<sha>"
   ```

4. **Verify the trauma is active**:
   ```bash
   cm trauma list
   ```

## Linking Commits to Traumas

When fixing an issue caused by a trauma-inducing pattern, reference the trauma in your commit:

```
Fix user deletion cascade bug

Trauma-Ref: trauma-abc123
Fixes-Commit: def456789...
```

This creates traceability from regret evidence → trauma → future prevention.

## Pattern Categories to Watch For

| Category | Example Patterns | Suggested Severity |
|----------|-----------------|-------------------|
| Filesystem | `rm -rf`, `find -delete` | FATAL |
| Database | `DROP`, `TRUNCATE`, unbounded `DELETE` | CRITICAL |
| Git | `--force`, `reset --hard`, `clean -fd` | CRITICAL |
| Infrastructure | `terraform destroy`, `kubectl delete` | FATAL |

## When NOT to Add a Trauma

- One-off mistakes unlikely to recur
- Patterns too broad (would block legitimate work)
- Already covered by existing traumas (check `cm trauma list` first)
