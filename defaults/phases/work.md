You are running the WORK phase of a multi-session backlog plan. The work
phase is interactive — you drive task selection with the user's input,
implement the chosen task, and record results.

## Required reads

Read the following files in order:

1. `{{PROJECT}}/README.md` — project conventions, architecture, build/test
   commands, and gotchas.
2. `{{PLAN}}/backlog.md` — the current task backlog
3. `{{PLAN}}/memory.md` — distilled learnings from prior sessions
4. `{{PLAN}}/related-plans.md` — declared peer-project relationships
   (only if the file exists)

**Placeholder note:** any file you {{TOOL_READ}} inside this project (READMEs,
backlog files, etc.) may contain literal `{{PROJECT}}`, `{{DEV_ROOT}}`,
or `{{PLAN}}` placeholder tokens. Substitute them mentally with the
absolute paths from this prompt before passing the path to the {{TOOL_READ}} tool.

## Related plans

{{RELATED_PLANS}}

Use this list for situational awareness while picking tasks. Do NOT read
sibling/parent/child backlogs or memories directly — cross-plan propagation
is the triage phase's job via dispatched subagents.

## Coding style

`{{ORCHESTRATOR}}/fixed-memory/` holds universal coding-style
reference material. Treat it like this:

- **At the moment you are about to write or modify code**, and not
  before, check `fixed-memory/`:
  - Always read `{{ORCHESTRATOR}}/fixed-memory/coding-style.md`
    — it contains the universal rules that apply to any language.
  - Also read `{{ORCHESTRATOR}}/fixed-memory/coding-style-<lang>.md`
    for whichever language you are about to touch, if such a file
    exists (e.g. `coding-style-rust.md` for Rust). If no file matches
    the language, there is no language-specific guidance for it —
    carry on with just the universal rules.
- If a task involves **no code** (pure docs, planning, backlog
  triage), skip this section entirely.
- If a task touches **multiple languages**, read each matching file
  before touching that language.

The plan does not tell you which language files apply — look at the
code you are about to change and pick from `fixed-memory/` yourself.

## Behavior

1. Display a summary of the current backlog. For each task, show title,
   status (`not_started` / `in_progress` / `done` / `blocked`), and
   priority.

2. Ask the user: "Any input on which task to work on next? If yes, name
   it; otherwise I'll pick the best next task." Wait for their response.

3. If the user named a task, work on that task. Otherwise pick the best
   next task — consider dependencies, priority, momentum, and fresh
   learnings from memory. Consider cross-plan awareness from the Related
   plans block above when judging relevance.

4. Implement the task. Respect any plan-specific commands, constraints,
   or conventions that appear AFTER this shared instructions block (added
   by the per-plan thin prompt, if present).

5. Verify the work: run tests, check outputs, inspect state. Do not
   declare done without evidence.

6. Review `.gitignore`. If the work introduced generated files, build
   artifacts, secrets, or other files that should not be version-
   controlled, add appropriate patterns to `.gitignore`.

7. Update the task's status and record results in `{{PLAN}}/backlog.md`.
   This has two required parts — do both, in this order:

   - **First, flip the `Status:` line.** Change the task's `Status:` line
     from `not_started` or `in_progress` to `done` (or to `blocked` with
     a brief reason in the status value). This step is required, not
     optional — a stale `Status:` line misleads triage into treating a
     finished task as still open, causing duplicate work.
   - **Then, write a `Results:` block** beneath the task describing what
     was done, what worked, what didn't, and what this suggests next.

8. **Commit your source-file changes yourself.** The orchestrator's
   `git-commit-work` step only commits the plan-state files inside
   `{{PLAN}}/` (backlog.md, phase.md, etc.). Any edits you made to
   source code, tests, docs, or configuration outside `{{PLAN}}/` must
   be staged and committed by you before the work phase ends —
   otherwise the next phase will advance with your work sitting
   unstaged in the working tree, where it can be silently lost across
   subsequent reflect/triage cycles.

   Before writing `analyse-work` to `phase.md`, run `git status` from
   the project root and confirm the working tree contains only the
   plan-state files you intend the orchestrator to commit. If
   `git status` shows any other modified, added, or untracked file
   that was part of this task, commit those changes now with a
   descriptive message. Cross-check the file list against any source
   changes you described in the task's `Results` block — every claimed
   path should appear in your own commit, not be left dangling.

9. Write `analyse-work` to `{{PLAN}}/phase.md`.

10. Stop. Do NOT pick another task. Do NOT reflect. Do NOT triage. One
    task per work phase — fresh context for reflection is more valuable
    than momentum.

    **Do NOT write session log entries.** The analyse-work phase handles
    session logging by examining the actual git diff — this produces a
    more accurate record than self-reporting.
