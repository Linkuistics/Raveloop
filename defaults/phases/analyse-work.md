You are running the ANALYSE-WORK phase of a multi-session backlog plan.
This phase runs headlessly immediately after the interactive work phase
exits. Its job is to examine the actual changes made during the work
session and produce two outputs: a session log entry and a git commit
message.

You are analysing what happened from the ground truth (the diff), not
from an LLM's self-report. The diff is the authoritative record of what
changed.

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

6. Determine the session number by counting existing `### Session`
   headings in `{{PLAN}}/session-log.md` (if it exists), then add one.

7. Write `{{PLAN}}/latest-session.md`, **OVERWRITING any prior content**.
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

8. Write `{{PLAN}}/commit-message.md` with a git commit message:

   ```
   <title — imperative mood, max 72 chars, summarises the change>

   <body — what was done and why, 2-5 lines>
   ```

   The title should be specific enough to be useful in `git log --oneline`.
   Do not include plan or phase metadata in the commit message — the
   git history provides that context.

9. Write `git-commit-work` to `{{PLAN}}/phase.md`.

10. Stop.

## Output format

After completing all writes, print nothing. The driver displays the
commit message. Do not mention phase.md.
