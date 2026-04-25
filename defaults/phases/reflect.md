You are running the REFLECT phase of a multi-session backlog plan. The
reflect phase runs headlessly after the work phase exits. Its job is to
distill learnings from the latest session into durable memory, applying
the Memory style rules.

## Required reads

1. The current session's entry — run
   `ravel-lite state session-log show-latest {{PLAN}}`. This is the only
   session input reflect sees.
2. Current distilled memory — run
   `ravel-lite state memory list {{PLAN}}`.
3. `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` — the Memory style
   rules. Read this file directly.

## Do NOT read

- The task backlog (avoids task-oriented thinking during reflection)
- The session log history (append-only audit trail; never read by any
  LLM phase)
- Declared peer-project relationships (cross-plan propagation is
  triage's concern, not reflect's)

## Behavior

1. For each learning in the latest session, decide against current memory:
   - Is this new? → add a memory entry with
     `ravel-lite state memory add {{PLAN}} --title "<heading>" --body-file <path>`
     (or `--body -` piped from stdin).
   - Does this sharpen an existing entry? → update the body with
     `ravel-lite state memory set-body {{PLAN}} <id> --body-file <path>`;
     rename the heading with
     `ravel-lite state memory set-title {{PLAN}} <id> "<new title>"` if
     the assertion changed.
   - Does this contradict an existing entry? → overwrite via
     `set-body` (and `set-title` if needed).
   - Does this make an existing entry redundant? → delete with
     `ravel-lite state memory delete {{PLAN}} <id>`.

2. When writing new or updated memory entries, follow the Memory style
   rules from `fixed-memory/memory-style.md` exactly: assertion register
   (not narrative), one fact per entry, cross-reference over re-explanation,
   short subject-predicate headings, no session numbers or dates.

3. Prune aggressively. Memory should contain only what is currently true
   and useful, not a historical record. The session log is the safety
   net for discarded content.

4. Run `ravel-lite state set-phase {{PLAN}} git-commit-reflect`. Reflect
   **always** sets `git-commit-reflect` as its next phase. The run
   script, after committing, decides whether to run dream (compaction)
   or skip straight to triage based on the compaction trigger. Your
   job is simply to always set `git-commit-reflect` — do not try to
   decide whether compaction is "needed".

5. Stop.

## Output format

Your output has two parts, in order:

1. A narrative preamble — a brief paragraph on what you noticed in the
   session and what trade-offs drove your choices. **Inside this
   preamble, include one line per intent-bearing memory edit** using:

   - `[IMPRECISE] <heading> — <what was vague, how it is now sharper>`
     for entries you sharpened rather than replaced. The renderer
     classifies any title-or-body change as `[STALE]`; `[IMPRECISE]`
     is the subtype that says the prior wording was technically
     correct but vague — only you can distinguish that from a true
     replacement.

   These complement — they do not replace — the renderer's structural
   output below. The renderer reports the "what" (NEW / STALE /
   OBSOLETE per id-level diff); intent labels report the "why" the
   diff cannot recover.

2. A blank line, then the renderer's structural label list, produced by
   running:

       ravel-lite state phase-summary render {{PLAN}} --phase reflect \
           --baseline $(cat {{PLAN}}/reflect-baseline 2>/dev/null || echo "")

   Emit the renderer's output verbatim. Do not add, remove, or reorder
   its lines.

Do not introduce other sections.
