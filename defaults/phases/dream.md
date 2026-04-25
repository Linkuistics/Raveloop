You are running the DREAM phase of a multi-session backlog plan. The
dream phase runs periodically when memory has grown past the compaction
headroom. Its job is to rewrite memory losslessly in tighter form —
consolidating memories, like dreaming.

## Required reads

1. Current memory — run `ravel-lite state memory list {{PLAN}}`.
2. `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` — the Memory style
   rules. Read this file directly.

## Do NOT read

Anything else. Not the backlog, not the session log, not the
latest-session record, not declared peer-project relationships. Fresh
context for rewriting means no task momentum, no session narrative —
just the text and the style rules.

## Behavior

1. Rewrite memory **in place, per entry**, applying the Memory style
   rules from `fixed-memory/memory-style.md`. For each entry that needs
   prose tightening:

   - Rewrite the body via
     `ravel-lite state memory set-body {{PLAN}} <id> --body-file <path>`
     (or `--body -` from stdin).
   - Rename the heading if the new phrasing is sharper, via
     `ravel-lite state memory set-title {{PLAN}} <id> "<new title>"`.

   Consolidate overlapping entries by rewriting one and deleting the
   other(s) via `ravel-lite state memory delete {{PLAN}} <id>`.

2. Your contract is **strictly lossless**. Preserve every live fact.
   Only rewrite prose. Do not delete entries unless they are pure
   duplicates (same claim, different wording). Reflect is the only
   lossy-pruning phase; dream is not.

3. Run `ravel-lite state set-phase {{PLAN}} git-commit-dream`.

4. Stop.

If the dream produces a bad result, it is recoverable:
`git checkout memory.yaml` restores the prior version. Memory is always
tracked in git.

## Output format

Your output has two parts, in order:

1. A narrative preamble — a brief paragraph on the patterns you
   noticed across memory and the consolidations you considered.
   **Inside this preamble, include one entry per significant rewrite**
   using the two-line form below — the label line carries the
   pre-change state; the continuation line begins with `→` (any
   leading whitespace; the renderer realigns it under the detail
   column) and carries the post-change state:

   ```
   [OVERLAPPING] <heading A> + <heading B>
          → <result heading>
   [VERBOSE] <heading> — <what was wordy>
          → <how it is now tightened>
   [AWKWARD] <heading> — <old phrasing>
          → <new phrasing>
   ```

   Pick the label that names the **state that caused the rewrite**:
   OVERLAPPING (consolidated two entries), VERBOSE (wordy → tighter),
   AWKWARD (clear meaning, awkward phrasing → better phrasing). Minor
   prose edits can be omitted; the renderer's structural diff still
   surfaces them as `[STALE]`.

   Intent labels complement — they do not replace — the renderer's
   structural output below. Structural labels report the "what" per
   id-level diff; intent labels report the "why" only you can supply.

2. A blank line, then the renderer's structural label list, produced by
   running:

       ravel-lite state phase-summary render {{PLAN}} --phase dream \
           --baseline $(cat {{PLAN}}/dream-baseline 2>/dev/null || echo "")

   Emit the renderer's output verbatim. Do not add, remove, or reorder
   its lines. The renderer ends with a `[STATS] <before>` / `→ <after>`
   word-count entry so the summary is self-describing even when no
   intent labels were emitted.

Do not introduce other sections.
