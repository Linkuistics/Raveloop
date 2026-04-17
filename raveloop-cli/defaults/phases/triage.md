You are running the TRIAGE phase of a multi-session backlog plan. The
triage phase runs headlessly at the end of each cycle. Its job is to
review and adjust the task backlog based on what the cycle learned, and
to propagate learnings to related plans when warranted.

## Required reads

1. `{{PLAN}}/backlog.md` — the task backlog
2. `{{PLAN}}/memory.md` — distilled learnings

## Related plans

{{RELATED_PLANS}}

## Do NOT read

- `{{PLAN}}/session-log.md`
- `{{PLAN}}/latest-session.md`
- **Any file under a sibling, parent, or child plan directory.** Cross-plan
  awareness comes from the Related plans block above (paths only) and from
  dispatched subagents — never from direct reads of foreign plan content.

## Behavior

### Local triage

1. Review each task in `backlog.md`:
   - Still relevant?
   - Priority changed?
   - Needs splitting?

2. Add new tasks implied by learnings in `memory.md`.

3. **Delete completed tasks.** Remove any task with status `done`, and
   clear any "Completed Tasks" section entirely — heading and all.
   Reflect has already run and anything worth keeping is now in
   `memory.md`; the session-log entry is the durable record of what
   happened. The backlog is for work that still needs doing, and must
   never carry a standing "Completed" holding area between cycles.

4. Remove tasks that are no longer relevant (dependencies met, approach
   changed, out of scope).

5. Reprioritize based on what the cycle revealed.

6. **Scan task descriptions for embedded blockers.** A spike, validation
   step, or shared dependency buried inside one task's description is
   invisible to future work phases until that task runs — even when it
   could run in parallel today. Promote any such blocker to its own
   top-level task so it surfaces as executable work.

## Cross-plan subagent dispatch

For each related plan where learnings warrant propagation, **write** `{{PLAN}}/subagent-dispatch.yaml` containing one entry per target:

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

7. Write `git-commit-triage` to `{{PLAN}}/phase.md`.

8. Stop.

## Output format

After completing all writes, print a brief summary using this structure:

```
[DELETED] <task title> — done, captured in memory
[ADDED] <task title> — <why>
[PROMOTED] <blocker title> — extracted from <parent task>
[REPRIORITISED] <task title> — <old priority → new>
[REMOVED] <task title> — <why obsolete>
[DISPATCH] <kind>: <target path> — <summary>
[NO DISPATCH] <reason>
```

One line per action. Do not include any other commentary. Do not
mention phase.md.
