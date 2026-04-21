You are running the ANALYSE-WORK phase of a multi-session backlog plan.
This phase runs headlessly immediately after the interactive work phase
exits. Its job is to examine the actual changes made during the work
session, **commit every source-file change on behalf of the session**,
and produce a session log entry plus a git commit message for the
plan-state files.

You are analysing what happened from the ground truth (the diff), not
from an LLM's self-report. The diff is the authoritative record of what
changed.

## Work-tree snapshot

The orchestrator captured this snapshot after the work phase exited —
treat it as authoritative. Do **not** trust your own mental model of
what changed; use this block as the definitive list of paths to commit
or justify.

```
{{WORK_TREE_STATUS}}
```

## Required reads

1. `{{PLAN}}/backlog.md` — the task backlog, to understand what task
   was being worked on and its recorded results.

## Do NOT read

- `{{PLAN}}/memory.md` (not needed for summarisation)
- `{{PLAN}}/related-plans.md` (not relevant here)

## Behavior

1. Read `{{PLAN}}/work-baseline` to get the baseline commit SHA.

2. Run `git diff <baseline-sha> --stat` for an overview of what files
   changed.

3. Run `git diff <baseline-sha>` for the full diff. If the diff is very
   large, focus on the most significant changes rather than trying to
   process everything.

4. Read `{{PLAN}}/backlog.md` to understand what task was being worked
   on and what results were recorded.

5. **Safety-net: mark completed tasks as `done`.** Scan the backlog for
   every task whose `Results:` block is non-empty (anything other than
   `_pending_` or an empty marker) while its `Status:` line is still
   `not_started` or `in_progress`. For each such task, flip the
   `Status:` line to `done` and save the file. This is a post-condition
   check, not a judgement call — the diff is authoritative; if the work
   phase wrote a results block but forgot to flip the status, this step
   repairs that drift so triage sees the real state. If the work phase
   already flipped everything correctly, this step is a no-op.

6. **Commit the source-file changes.** The work phase no longer commits
   its own source edits — that is this phase's responsibility. Using the
   work-tree snapshot above as ground truth:

   - Stage every path outside `{{PLAN}}/` that appears in the snapshot
     with `git add <path>` (or `git add -A -- :!{{PLAN}}` if the set is
     large — but explicit paths are preferred).
   - Commit with a descriptive message in the imperative mood that
     summarises the session's code changes (not the plan-bookkeeping
     commit, which happens separately from `commit-message.md`).
   - If any path in the snapshot is intentionally **not** committed
     (e.g. a user scratch file, an accidental edit that should be
     reverted), you MUST name each such path explicitly in
     `latest-session.md` (next step) with a one-sentence justification.
     Unjustified uncommitted paths will trigger a TUI warning.

   Do not commit files inside `{{PLAN}}/` here — those are reserved for
   the subsequent `git-commit-work` script phase.

7. Determine the session number by counting existing `### Session`
   headings in `{{PLAN}}/session-log.md` (if it exists), then add one.

8. Write `{{PLAN}}/latest-session.md`, **OVERWRITING any prior content**.
   Use this format:

   ```
   ### Session N (YYYY-MM-DDTHH:MM:SSZ) — brief title
   - What was attempted
   - What worked, what didn't
   - What this suggests trying next
   - Key learnings or discoveries
   ```

   The timestamp is ISO 8601 UTC with seconds precision. Obtain with:
   `date -u '+%Y-%m-%dT%H:%M:%SZ'`

   Base the entry on the actual diff and the backlog results, not
   assumptions. Be specific about what files were changed and why.

9. Write `{{PLAN}}/commit-message.md` with a git commit message for the
   **plan-state commit** (the one `git-commit-work` will make next). This
   is distinct from the source-file commit you made in step 6 and should
   narrate the plan bookkeeping: safety-net status flips, backlog
   `Results:` additions, phase.md transitions. Keep it tight:

   ```
   <title — imperative mood, max 72 chars, summarises plan-state updates>

   <body — what was done and why, 2-5 lines>
   ```

   The title should be specific enough to be useful in `git log --oneline`.
   Do not include plan or phase metadata in the commit message — the
   git history provides that context.

10. Run `ravel-lite state set-phase {{PLAN}} git-commit-work`.

11. Stop.

## Output format

After completing all writes, print nothing. The driver displays the
commit message. Do not mention phase.md.
