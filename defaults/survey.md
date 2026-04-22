# Plan status survey

You are producing a multi-project plan status overview for a developer
who wants to know which plan to run through `ravel-lite run` next.
Plans follow the Ravel-Lite convention: a directory is a plan iff it
contains `phase.md`; siblings `backlog.md` and `memory.md` hold task
state and distilled learnings.

Below (after the horizontal rule) are all discovered plans. For each
plan you have:

- project (basename of the nearest ancestor directory containing `.git`)
- plan (plan directory basename)
- phase (contents of `phase.md`)
- backlog (contents of `backlog.md`, or `(missing)` if absent)
- memory (contents of `memory.md`, or `(missing)` if absent)

## Your output

Respond with a single YAML document matching this schema — nothing
else. No prose preamble, no Markdown code fences, no commentary.
The calling tool parses your response and owns all final formatting.

```
plans:
  - project: <string>             # project basename, as provided above
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
    # per-status totals yourself — the tool fills them in from each
    # plan's parsed `backlog.md`. Do not emit `task_counts` in your
    # response; the tool injects it post-parse.

cross_plan_blockers:
  - blocked: <project>/<plan>     # plan that is blocked
    blocker: <project>/<plan>     # plan whose output unblocks it
    rationale: |                  # one or two sentences; use `|` block scalar for safety
      Why this is blocked and what would unblock it. Free prose.
      May span multiple lines.

parallel_streams:
  - name: <string>                # short descriptive name, e.g. "Critical path"
    plans:                        # plans that make up this stream
      - <project>/<plan>
      - <project>/<plan>
    rationale: |                  # why these belong in one stream; note any
      intra-stream sequencing (gates, dependencies) vs fully
      concurrent work. Explain why this stream can run concurrently
      with other streams.

recommended_invocation_order:
  - plan: <project>/<plan>        # plan to invoke next via ravel-lite run
    order: <int>                  # rank (1 = highest priority). Entries
                                  # sharing an `order` are mutually
                                  # parallelisable. See rules below.
    rationale: |                  # one or two sentences of rationale
      Why run this next, grounded in the files above.
```

## Rules for populating the schema

- Include EVERY discovered plan in the `plans` list. Do not omit any.
- Sort `plans` by project, then plan name.
- `notes` is terse (one short phrase). Use it to flag things like
  "2 unprocessed dispatches", "backlog.md missing", or "stale
  pre-pivot framing". Leave it as an empty string if there's nothing
  specific to note.
- A plan with `backlog.md` missing: counts are all 0, `notes: backlog.md missing`.
- `cross_plan_blockers`: entries where blocker and blocked are
  DIFFERENT plans. Include both same-project and cross-project
  dependencies — the survey is the place to see them all at once.
  A plan's dependency on itself (or on a task within itself) belongs
  in that plan's backlog, not here. Omit the key or return `[]` if
  no cross-plan dependencies exist.
- `parallel_streams`: group plans into sets whose work can proceed
  concurrently with the other sets. Each stream may itself be a
  sequential chain (e.g. gate-task → implementation), but streams do
  not block each other. Every recommended plan should belong to some
  stream. Omit the key or return `[]` if all work is one linear chain.
- `recommended_invocation_order`: up to five entries, highest priority
  first. Each entry must include an integer `order` field:
    - Entries sharing the same `order` value are mutually
      parallelisable — running any subset concurrently is safe.
    - Smaller `order` = higher priority. Orders usually start at 1.
    - Numbers do NOT need to be contiguous; gaps are fine.
    - List the entries in order of `order` (ascending), and WITHIN a
      shared `order`, list the most-unblocking entry first (that
      secondary ordering becomes the list position you emit).
  Priority order for assigning `order`:
    1. Plans with unprocessed `## Received` items whose triage unblocks
       other plans on the critical path.
    2. Plans with `not_started` tasks marked `P1` and no dependencies.
    3. Independent research or literature-survey plans (cheap to run,
       often unblocked).
  Skip plans whose only remaining work is `done` or `blocked` on
  external input.

## Hard rules

- Respond with YAML ONLY. No preamble, no conclusion, no code fences.
- Use `|` block scalars for every multiline prose field.
- Do not speculate beyond what the files say.
- When a file is missing, surface it in `notes`; do not infer contents.
