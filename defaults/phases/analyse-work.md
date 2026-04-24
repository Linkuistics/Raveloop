You are running the ANALYSE-WORK phase of a multi-session backlog plan.
This phase runs headlessly immediately after the interactive work phase
exits. Its job is to examine the actual changes made during the work
session, produce a session log entry, and author a `commits.yaml` spec
that the subsequent `git-commit-work` script phase will apply to the
work tree. You never issue `git commit` yourself — staging and
committing is entirely the script phase's responsibility.

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

## Backlog transitions since baseline

The orchestrator computed this diff between `backlog.yaml` at the work
baseline and `backlog.yaml` right now. Use it to understand which
backlog entries moved during the session — what was completed, added,
deleted, or reprioritised. This is authoritative — do **not** re-derive
it from the full diff.

```
{{BACKLOG_TRANSITIONS}}
```

Caveat: the baseline is the reflect commit of the previous cycle, so
task additions/deletions from the previous cycle's triage may also
appear in this block. Treat entries outside the current session's focus
as baseline context.

**Do not author commit titles or bodies around these task ids.** Commit
messages describe the work that happened in the tree, not the backlog
bookkeeping that framed it — see step 9 for the full rule.

## Required reads

1. The task backlog — run `ravel-lite state backlog list {{PLAN}}`. You
   need to understand what task was being worked on and its recorded
   results.

## Do NOT read

- The memory (not needed for summarisation)
- Declared peer-project relationships (not relevant here)

## Behavior

1. Read `{{PLAN}}/work-baseline` to get the baseline commit SHA.
   `work-baseline` is a plain text file, not a state-CLI-managed one.

2. Run `git diff <baseline-sha> --stat` for an overview of what files
   changed.

3. Run `git diff <baseline-sha>` for the full diff. If the diff is very
   large, focus on the most significant changes rather than trying to
   process everything.

4. Inspect the backlog (from step 1's required read) to understand what
   task was being worked on and what results were recorded.

5. **Safety-net: repair stale task statuses.** Run
   `ravel-lite state backlog repair-stale-statuses {{PLAN}}`. The verb
   flips `in_progress` tasks with non-empty results to `done` and
   unblocks `blocked` tasks whose structural dependencies are all now
   `done`. It emits a YAML report of any repairs applied — if a repair
   occurred and it materially changed the backlog shape, note the
   *kind* of repair in the relevant commit body (step 9) without
   naming specific task ids. If no drift is present this step is a
   no-op.

6. **Review the work tree; revert accidental edits.** You do not
   issue `git add` or `git commit` in this phase — the subsequent
   `git-commit-work` script phase stages and commits everything per
   the `commits.yaml` spec you write in step 9. Your job here is only
   to clean the tree of anything you do NOT want committed:

   - Using the `{{WORK_TREE_STATUS}}` snapshot as ground truth,
     identify any accidental edits — a debug print left in, a stray
     save in a file you didn't mean to touch, an experimental change
     the session decided against.
   - Revert tracked accidentals with `git checkout -- <path>` and
     remove untracked accidentals with `rm <path>`.
   - Anything still in the tree after this step will end up committed
     by `git-commit-work`, so be deliberate.

   If something is intentionally uncovered by any spec entry in step
   9 (rare), name it in the session record with a one-sentence
   justification. Uncommitted residue after `git-commit-work` applies
   the spec triggers a TUI warning.

7. Determine the session number by counting records returned from
   `ravel-lite state session-log list {{PLAN}}`, then add one. Record
   IDs themselves are free-form; sequential numbering in the entry body
   is the convention.

8. Write the session record via
   `ravel-lite state session-log set-latest {{PLAN}} --id <session-id> --timestamp <ts> --phase work --body-file <path>`
   where `<session-id>` is a short slug like `session-N`,
   `<ts>` is ISO 8601 UTC with seconds precision
   (`date -u '+%Y-%m-%dT%H:%M:%SZ'`), and `<path>` is a temp file whose
   content follows this layout:

   ```
   ### Session N (YYYY-MM-DDTHH:MM:SSZ) — brief title
   - What was attempted
   - What worked, what didn't
   - What this suggests trying next
   - Key learnings or discoveries

   ## Hand-offs (omit this section entirely when there are none)

   ### <hand-off title>
   - Problem being solved
   - Settled design decisions, each with a one-sentence rationale
   - Reference examples (file paths, line numbers)
   - Dependencies
   ```

   Base the entry on the actual diff and the backlog results, not
   assumptions. Be specific about what files were changed and why.

   **Hand-off convention.** A hand-off is a forward-looking design
   that this session settled but did NOT implement — a Q&A that
   resolved an approach, a reference example identified for a later
   task, a rejected alternative worth recording. Hand-offs must
   survive analyse-work → triage even when the implementing task is
   deleted next cycle. Record each hand-off by whichever channel fits:

   - **Preferred: promote directly to a new backlog task.** When the
     design is concrete enough to be picked up by a future work
     cycle, run
     `ravel-lite state backlog add {{PLAN}} --title "<title>" --category <cat> --description-file <path>`
     with `Status: not_started` (the default) and a description that
     **inlines** the settled design — not a one-liner pointer.
     Include: the problem being solved, each decision with a
     one-sentence rationale, reference examples (file paths, line
     numbers), and dependencies. Target 10–40 lines; enough that
     triage can promote without rereading the whole diff.

   - **Fallback: record in the `## Hand-offs` section above AND write
     a `[HANDOFF] <title>` hand-off block on the completing task
     via `ravel-lite state backlog set-handoff {{PLAN}} <task-id> --body-file <path>`.**
     Use this when the design is only partially settled. Triage mines
     hand-off blocks from completed tasks before deleting them (see
     `defaults/phases/triage.md` step 3).

9. Write `{{PLAN}}/commits.yaml` with an ordered list of the commits
   you want `git-commit-work` to apply. `commits.yaml` is a one-shot
   scratch file, not a state-CLI-managed one, so write it directly;
   the script phase reads it, applies each entry in order, and
   removes the file afterwards.

   Shape:

   ```yaml
   commits:
     - paths: [<git pathspec>, <git pathspec>, ...]
       message: |
         <title — imperative mood, max 72 chars>

         <optional body, 2-5 lines>
   ```

   Pathspecs are passed straight to `git add`, so standard git
   features work: literal paths, globs (`src/**`), and exclusions
   (`:!src/generated/`). Use `"."` to mean "everything in the
   subtree".

   **One commit per cycle is the default.** Unless the session
   genuinely spanned two independent concerns, write a single entry
   covering the whole subtree. The commit then spans source, docs,
   config, AND plan-state files together:

   ```yaml
   commits:
     - paths: ["."]
       message: |
         Wire the greeting path through the new renderer

         Renames the intermediate struct to match its new role and
         updates the two call sites. Plan-state updates for this
         session are included.
   ```

   Plan-state mutations (backlog status flip, results block, session
   log append) are implicit in every analyse-work cycle — they must
   NOT drive the commit title. The title is about what changed in
   the tree; bookkeeping is background noise.

   **Split only when the diff genuinely spans independent concerns.**
   If the session legitimately covered two unrelated tracks — a
   source fix in `src/` plus an unrelated docs update in `docs/`
   that would confuse a reader if bundled — split into multiple
   entries partitioned by top-level directory (`src/`, `docs/`,
   `tests/`, `defaults/`, `scripts/`, `{{PLAN}}/`). If a single file
   mixes independent concerns, that is a cue the work was scoped
   wrong; bundle it into one commit with an honest message and
   split along semantic lines next cycle.

   **Plan-state-only cycles.** If the only changed paths are under
   `{{PLAN}}/`, frame the title around the *shape* of the plan-state
   mutation — e.g. `Update backlog description and reprioritise`,
   `Promote a hand-off to the backlog`, `Reprioritise backlog and
   archive a hand-off`. "Record results and flip status" is not a
   useful title — every cycle does that — so describe what actually
   moved instead.

   **Title rules** (apply to every entry):

   - Imperative mood, max 72 chars.
   - Describe what changed in the tree — the function added, the
     behaviour fixed, the file renamed, the shape of plan-state
     movement.
   - Do NOT reference backlog task ids (`fix-work-baseline`),
     session numbers (`session 47`), phase names
     (`during analyse-work`), or plan-bookkeeping framing
     (`mark task X done, record results`).
   - If the only way to describe the change is by task id, the
     commit scope is wrong — split the underlying work along
     semantic lines first.

   **Coverage.** Every path in the work-tree snapshot must be
   reachable from at least one entry's `paths` list (or intentionally
   reverted in step 6). Uncommitted residue after the script phase
   applies the spec triggers a TUI warning.

   **Fallback.** If you omit `commits.yaml` entirely or the file is
   malformed, `git-commit-work` falls back to a single catch-all
   commit of the whole subtree under the default message
   `run-plan: work ({{PLAN}})`. This is a safety net, not an
   intended path — always write the spec.

10. Run `ravel-lite state set-phase {{PLAN}} git-commit-work`.

11. Stop.

## Output format

After completing all writes, print nothing. The driver displays each
commit's subject line as the script phase applies the spec.
