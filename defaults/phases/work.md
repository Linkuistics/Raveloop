You are running the WORK phase of a multi-session backlog plan. The work
phase is interactive — you drive task selection with the user's input,
implement the chosen task, and record results.

## Required reads

Read the following in order:

1. `{{PROJECT}}/README.md` — project conventions, architecture, build/test
   commands, and gotchas. Use the `{{TOOL_READ}}` tool.
2. The current task backlog — run `ravel-lite state backlog list {{PLAN}}`.
3. Distilled memory — run `ravel-lite state memory list {{PLAN}}`.
4. Declared peer-project relationships — run
   `ravel-lite state related-components list --plan {{PLAN}}` (empty output
   is fine — it means this plan has no declared peers).

**Placeholder note:** any file you {{TOOL_READ}} inside this project
(READMEs, etc.) may contain literal `{{PROJECT}}`, `{{DEV_ROOT}}`, or
`{{PLAN}}` placeholder tokens. Substitute them mentally with the
absolute paths from this prompt before passing the path to the
{{TOOL_READ}} tool.

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
   priority. The backlog YAML from step 2 carries every field you need —
   do not re-read `backlog.md`.

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

7. Update the task's status and record results in the backlog. This has
   two required parts — do both, in this order:

   - **First, flip the status.** Run
     `ravel-lite state backlog set-status {{PLAN}} <task-id> done` (or
     `blocked --reason "<short reason>"`). This step is required, not
     optional — a stale status misleads triage into treating a finished
     task as still open, causing duplicate work.
   - **Then, write a `Results:` block.** Run
     `ravel-lite state backlog set-results {{PLAN}} <task-id> --body-file <path>`
     where `<path>` is a temp file containing the markdown body, or pipe
     the body via stdin with `--body -`. The body describes what was
     done, what worked, what didn't, and what this suggests next.

8. **Do NOT commit source-file changes yourself.** The analyse-work
   phase that runs immediately after this one is responsible for
   committing everything you edited (source, tests, docs, config — any
   path outside `{{PLAN}}/`). The orchestrator captures a `git status`
   snapshot the moment this phase exits and feeds it into the
   analyse-work prompt as authoritative input, so anything you leave
   uncommitted will be seen and committed (or explicitly justified).

   You are free to run `git status` / `git diff` for your own
   orientation, but do not stage or commit anything. Leaving the tree
   dirty for analyse-work is the expected hand-off.

9. Run `ravel-lite state set-phase {{PLAN}} analyse-work`.

10. Stop. One task per work phase is the default — fresh context for
    reflection is more valuable than momentum — so do not pick another
    task on your own initiative, reflect, or triage. If the user
    explicitly requested multiple tasks in step 2, honour that request:
    complete each one (repeating steps 4-7 per task) before the single
    step 9 transition, then stop. Do not volunteer additional work
    beyond what the user asked for.

    **Do NOT write session log entries.** The analyse-work phase handles
    session logging by examining the actual git diff — this produces a
    more accurate record than self-reporting.
