You are running the TRIAGE phase of a multi-session backlog plan. The
triage phase runs headlessly at the end of each cycle. Its job is to
review and adjust the task backlog based on what the cycle learned, and
to propagate learnings to related plans when warranted.

## Required reads

1. The task backlog — run `ravel-lite state backlog list {{PLAN}}`.
2. Distilled memory — run `ravel-lite state memory list {{PLAN}}`.

## Related plans

{{RELATED_PLANS}}

## Do NOT read

- The session log history
- The latest-session record
- **Any file under a sibling, parent, or child plan directory.** Cross-plan
  awareness comes from the Related plans block above (paths only) and from
  dispatched subagents — never from direct reads of foreign plan content.

## Behavior

### Local triage

**Preflight.** Run `ravel-lite state backlog lint-dependencies {{PLAN}}`
to detect prose mentions of task ids that are missing from the
structured `dependencies:` field. Use the output to inform reconciliation
during the steps below; do not re-scan prose manually. Reconcile flagged
tasks with
`ravel-lite state backlog set-dependencies {{PLAN}} <id> --deps <id1,id2>`
(or `--deps ""` to clear). An id mentioned in prose is not always a true
dependency — treat the report as advisory, not authoritative.

1. Review each task in the backlog:
   - Still relevant?
   - Priority changed?
   - Needs splitting?

   Apply changes via the appropriate verb:
   - `ravel-lite state backlog set-title {{PLAN}} <id> "<new title>"`
   - `ravel-lite state backlog set-status {{PLAN}} <id> <status>` (or
     `blocked --reason "<reason>"`)
   - `ravel-lite state backlog reorder {{PLAN}} <id> --before <other-id>`
     or `--after <other-id>` to reprioritise.

2. Add new tasks implied by learnings in memory via
   `ravel-lite state backlog add {{PLAN}} --title "<title>" --category <cat> --description-file <path>`
   (optionally `--dependencies <id1,id2>`). When the task description
   names another task id in prose, pass `--dependencies <ids>` at add
   time; the preflight linter (above) catches any drift introduced by
   later edits.

3. **Mine completed tasks for hand-offs, then delete them.** List
   candidates with `ravel-lite state backlog list {{PLAN}} --has-handoff`
   (tasks carrying an explicit hand-off block) and inspect each done
   task's `Results:` body for `[HANDOFF]` markers or labelled
   `Hand-offs:` / `Followups:` sections. For each hand-off found:

   - **Promote to a new top-level backlog task** when the settled
     design is concrete — run
     `ravel-lite state backlog add {{PLAN}} --title "<title>" --category <cat> --description-file <path>`
     with the inlined decision content verbatim in the description,
     and include a `[PROMOTED] <hand-off title> — <one-line reason>`
     line in your narrative preamble.
   - **Archive to memory** when the design is strategic but not yet
     concrete enough to be a standalone task — run
     `ravel-lite state memory add {{PLAN}} --title "<heading>" --body-file <path>`
     capturing the design intent and rationale, and include an
     `[ARCHIVED] <hand-off title> — <one-line reason>` line in your
     narrative preamble.

   After every hand-off is extracted, clear the hand-off block with
   `ravel-lite state backlog clear-handoff {{PLAN}} <task-id>`, then
   delete the task with `ravel-lite state backlog delete {{PLAN}} <task-id>`
   (use `--force` if the task is referenced as a dependency elsewhere).
   Reflect has already run and anything worth keeping is now in memory;
   the session-log entry is the durable record of what happened. The
   backlog is for work that still needs doing, and must never carry
   a standing "Completed" holding area between cycles.

4. Remove tasks that are no longer relevant (dependencies met, approach
   changed, out of scope) via `ravel-lite state backlog delete`.

5. Reprioritize based on what the cycle revealed using
   `ravel-lite state backlog reorder`.

6. **Scan task descriptions for embedded blockers.** A spike, validation
   step, or shared dependency buried inside one task's description is
   invisible to future work phases until that task runs — even when it
   could run in parallel today. Promote any such blocker to its own
   top-level task (via `state backlog add`) so it surfaces as
   executable work, and include a
   `[BLOCKER] <new task title> — extracted from <parent task title>`
   line in your narrative preamble.

## Cross-plan subagent dispatch

For each related plan where learnings warrant propagation, **write**
`{{PLAN}}/subagent-dispatch.yaml` directly (this is a one-shot scratch
file, not a state-CLI-managed one) containing one entry per target:

```yaml
dispatches:
  - target: /absolute/path/to/related/plan
    kind: child
    summary: |
      One to three paragraphs describing the learnings and
      suggested backlog/memory updates for the target plan.
```

Rules:
- Use absolute paths for targets
- Use `|` (block scalar) for multi-line summaries
- Omit the file entirely if there are no dispatches
- Do **not** attempt to dispatch anything yourself — the driver reads this file after you exit and handles dispatch
- For each dispatch entry written, include a
  `[DISPATCH] <kind>: <target plan name> — <one-line summary>` line in
  your narrative preamble.

7. Run `ravel-lite state set-phase {{PLAN}} git-commit-triage`.

8. Stop.

## Output format

Your output has two parts, in order:

1. A narrative preamble — a brief paragraph on what patterns you saw
   across the backlog and what drove reprioritisations or hand-off
   decisions. **Inside this preamble, include one line per
   intent-bearing action**, using the formats specified above:

   - `[PROMOTED] <hand-off title> — <reason>` (step 3)
   - `[ARCHIVED] <hand-off title> — <reason>` (step 3)
   - `[BLOCKER] <new task title> — extracted from <parent task title>` (step 6)
   - `[DISPATCH] <kind>: <target plan name> — <summary>` (cross-plan dispatch)

   These complement — they do not replace — the renderer's structural
   output below. Intent labels capture the "why" the diff cannot
   recover; structural labels capture the "what".

2. A blank line, then the renderer's structural label list, produced by
   running:

       ravel-lite state phase-summary render {{PLAN}} --phase triage \
           --baseline $(cat {{PLAN}}/triage-baseline 2>/dev/null || echo "")

   Emit the renderer's output verbatim. Do not add, remove, or reorder
   its lines.

Do not introduce other sections.
