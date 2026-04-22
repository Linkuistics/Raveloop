# Plan status survey — incremental mode

You are producing a delta update to an existing multi-project plan
status survey. A prior survey was produced previously; since then the
calling tool has computed content hashes for every plan's state files
and identified which plans changed, which were added, which remain
unchanged, and which were removed.

Your job is to analyse only the **changed or added** plans and return
a YAML document covering those rows plus the annotation sections
(`cross_plan_blockers`, `parallel_streams`,
`recommended_invocation_order`). The calling tool will splice your
delta rows into the prior survey's unchanged rows, producing the
merged final result. Unchanged rows are NOT your responsibility —
do not emit rows for them; the tool carries them forward verbatim.

Below (after the horizontal rule) you will find, in order:

1. **Prior survey (context)** — the full prior YAML. Use this as the
   reference for current cross-plan state. Rows in this YAML whose
   plan does NOT appear in "Changed or added plans" are unchanged;
   you may reference them in annotation sections but must not emit
   new rows for them.
2. **Plans removed since prior** — plan identifiers (`project/plan`)
   that were in the prior survey but no longer exist. Their rows must
   disappear from the merged survey (the tool handles that). Any
   blocker, stream, or recommendation that referenced a removed plan
   must be dropped or rewritten in your annotation sections.
3. **Changed or added plans** — full `phase.md`, `backlog.md`, and
   `memory.md` contents for every plan you must analyse. For each
   listed plan, emit exactly one row in your `plans:` array.

## Your output

Respond with a single YAML document matching this schema — nothing
else. No prose preamble, no Markdown code fences, no commentary.

```
schema_version: 1
plans:
  - project: <string>             # project basename (must match a
                                  # "Changed or added plans" entry)
    plan: <string>                # plan directory basename
    phase: <string>               # raw contents of phase.md, trimmed
    unblocked: <int>              # backlog tasks that are not_started AND have no unmet deps
    blocked: <int>                # backlog tasks with status=blocked OR not_started with unmet deps
    done: <int>                   # see note below — prefer pre-populated task_counts.done
    received: <int>               # count of dispatches under `## Received` NOT yet promoted to numbered tasks
    notes: <string>               # short free-text cell; leave empty if nothing worth noting
    # NOTE: a `task_counts` object (total / not_started / in_progress /
    # done / blocked) is populated by the calling tool in Rust after
    # your response is parsed. You do NOT need to tally those raw
    # per-status totals yourself. Do not emit `task_counts` in your
    # response; the tool injects it post-parse.

cross_plan_blockers:
  - blocked: <project>/<plan>
    blocker: <project>/<plan>
    rationale: |
      Why this is blocked and what would unblock it.

parallel_streams:
  - name: <string>
    plans:
      - <project>/<plan>
      - <project>/<plan>
    rationale: |
      Why these belong in one stream.

recommended_invocation_order:
  - plan: <project>/<plan>
    order: <int>
    rationale: |
      Why run this next.
```

## Rules for populating the schema

- Your `plans` list must contain EXACTLY one row per plan listed in
  "Changed or added plans" — no fewer, no more. The tool rejects a
  delta that mutates any plan outside the declared set.
- Annotation sections (`cross_plan_blockers`, `parallel_streams`,
  `recommended_invocation_order`) should span ALL plans in the
  merged view, not just the delta. You have the prior survey and
  the delta inputs; combine both to produce an up-to-date annotation
  set. Drop any reference to a plan listed under "Plans removed
  since prior".
- A plan with `backlog.md` missing: counts are all 0,
  `notes: backlog.md missing`.
- `notes` is terse (one short phrase). Use it to flag things like
  "2 unprocessed dispatches", "backlog.md missing", or "stale
  pre-pivot framing". Leave it as an empty string if there's nothing
  specific to note.
- `recommended_invocation_order`: up to five entries, highest
  priority first. Each entry must include an integer `order` field.
  Entries sharing the same `order` are mutually parallelisable. See
  the cold-mode prompt (`survey.md`) for the full priority rubric;
  those rules still apply here.

## Hard rules

- Respond with YAML ONLY. No preamble, no conclusion, no code fences.
- Use `|` block scalars for every multiline prose field.
- Emit `schema_version: 1` at the top — the tool validates this.
- Include every changed/added plan listed below and no others.
- Do not emit rows for unchanged plans; the tool carries them forward.
