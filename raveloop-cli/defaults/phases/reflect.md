You are running the REFLECT phase of a multi-session backlog plan. The
reflect phase runs headlessly after the work phase exits. Its job is to
distill learnings from the latest session into durable memory, applying
the Memory style rules.

## Required reads

1. `{{PLAN}}/latest-session.md` — the current session's entry (the only
   session input reflect sees)
2. `{{PLAN}}/memory.md` — current distilled learnings
3. `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` — the Memory
   style rules

## Do NOT read

- `{{PLAN}}/backlog.md` (avoids task-oriented thinking during reflection)
- `{{PLAN}}/session-log.md` (append-only audit trail; never read by any
  LLM phase)
- `{{PLAN}}/related-plans.md` (cross-plan propagation is triage's concern,
  not reflect's)

## Behavior

1. For each learning in the latest session, decide against current memory:
   - Is this new? → add a memory entry
   - Does this sharpen an existing entry? → update it with more precision
   - Does this contradict an existing entry? → replace with the corrected
     understanding
   - Does this make an existing entry redundant? → remove the redundant one

2. When writing new or updated memory entries, follow the Memory style
   rules from `fixed-memory/memory-style.md` exactly: assertion register
   (not narrative), one fact per entry, cross-reference over re-explanation,
   short subject-predicate headings, no session numbers or dates.

3. Prune aggressively. `memory.md` should contain only what is currently
   true and useful, not a historical record. `session-log.md` is the
   safety net for discarded content.

4. Write `git-commit-reflect` to `{{PLAN}}/phase.md`. Reflect
   **always** writes `git-commit-reflect` as its next phase. The run
   script, after committing, decides whether to run dream (compaction)
   or skip straight to triage based on the compaction trigger. Your
   job is simply to always write `git-commit-reflect` — do not try to
   decide whether compaction is "needed".

5. Stop.

## Output format

After completing all writes, print a brief summary using this structure:

```
[ADDED] <heading> — <one-line description>
[SHARPENED] <heading> — <what changed>
[REPLACED] <heading> — <old → new>
[REMOVED] <heading> — <why>
```

One line per memory action. Do not include any other commentary — just
the action list. Do not mention phase.md.
