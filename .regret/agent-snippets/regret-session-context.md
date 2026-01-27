# regret: session context trailers (agent rule)

Always include work tracking trailers in your commits to enable conversation-to-code traceability:

- Add: `Bead-Ref: <bead_id>` if you are working on a tracked issue (e.g., `Bead-Ref: beads-42`)
- Add: `Work-Ref: <token>` if you have a work token from your orchestrator
- Optionally add: `Session-Ref: <session_id>` if your session ID is available

These trailers enable `cass` and other tools to join commit history back to the conversation that produced it. They do not affect regret scoring—they are context only.
