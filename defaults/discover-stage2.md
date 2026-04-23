# Discovery — Stage 2: Infer Cross-Project Edges

You are given a collection of per-project interaction-surface records.
Your task is to propose relationship edges between catalogued components
based on what their surfaces reveal.

## Edge kinds

The ontology below enumerates every relationship kind you may propose.
Use kind names exactly as listed; do not invent kinds. Each kind
declares its direction (**directed** — participant order is semantic,
so pick it intentionally; **symmetric** — participants carry no order,
sort them alphabetically) and its typical lifecycle scopes. Pick the
lifecycle that best matches the evidence you cite; if several apply,
prefer the tightest (e.g. `build` over `runtime` when the coupling
only holds at compile time).

{{ONTOLOGY_KINDS}}

## Decision tree

Walk these questions in order against each pair of components whose
surfaces share any signal. Propose an edge the moment one branch
matches with direct, cross-referenced evidence; skip the branch
otherwise. A pair may match more than one branch — §3.5 of the
ontology explicitly allows multiple `(kind, lifecycle)` edges on the
same pair.

1. **Runtime message exchange?** Overlapping `network_endpoints`
   with matching protocol/address.
   → `communicates-with @ runtime` when the two sides are peers with
     no clear client/server split; `calls @ runtime` when one side is
     plainly the client of an endpoint the other serves.
2. **Source generation into another tree?** One project's
   `produces_files` overlaps another's source-tree paths or
   `consumes_files`, and the coupling is ongoing (regenerated on
   change, not one-shot).
   → `generates @ codegen`. Use `scaffolds @ dev-workflow` only when
     the generation is explicitly one-shot initial scaffolding.
3. **Process spawning?** One project's `external_tools_spawned` names
   a binary owned by another catalog component.
   → `invokes` for one-shot CLI invocations (`dev-workflow`) or
     persistent child processes (`runtime`). Upgrade to
     `orchestrates` when the spawner manages the spawnee's lifecycle,
     state, and multi-step workflow — orchestration is stronger than
     invocation.
4. **Library dependency?** Package-manifest entries, import
   statements, or `consumes_files` referencing a manifest.
   → `depends-on` at the appropriate lifecycle (`build` for
     compile-time deps, `runtime` for loaded-at-runtime deps). Use
     `links-statically` / `links-dynamically` only when linkage
     specifics are in evidence. `has-optional-dependency` when the
     dependency is flagged optional; `provided-by-host` when the
     project expects the dependency to be present in the execution
     environment rather than bundled.
5. **In-process embedding?** One project runs the other in-process as
   a whole program (library embedding, WASM, subprocess-in-pipe) —
   distinct from dynamic linkage.
   → `embeds @ runtime`.
6. **Common external specification declared by both components?**
   Both reference implementing the same RFC / protocol / schema
   that neither component owns.
   → `co-implements @ design`. Use `conforms-to @ design` when one
     component owns the specification the other implements.
7. **Documentation relationship?** One project's purpose is to
   document another (docs repo, architecture notes, external user
   guide).
   → `describes @ design`.
8. **Test harness or fixture provider?** One project's primary
   purpose is to exercise another, or to provide test data / mocks /
   fixtures that the other's test suite loads.
   → `tests @ test` or `provides-fixtures-for @ test`.
9. **None of the above with direct evidence?**
   → No edge. Omission is the correct answer for weak overlaps.

## Insufficient signals (weak evidence-grade threshold)

Patterns too thin to support a proposal on their own. If the only
evidence you have falls into this list, either omit the edge or mark
it `evidence_grade: weak` with rationale explaining why the signal is
nevertheless informative.

- **Shared upstream dependencies.** Two components independently
  mentioning the same *third* catalog component in their
  `explicit_cross_project_mentions` is not evidence of an edge between
  those two. Many unrelated components share infrastructure
  dependencies.
- **Same programming language or ecosystem.** Both being Rust crates,
  Racket packages, Swift apps, or Node packages is not a relationship.
- **Generic or trivial file-glob overlap.** Patterns like `*.txt`,
  `**/*.rkt`, `~/.config/**`, or any whole-language source-tree glob
  are too broad to constitute file-level coupling. Require a specific,
  named file or a narrow glob whose match set is plausibly produced
  by one project and consumed by another.
- **Same external tools alone.** Both components spawning `git` or
  `bash` is not evidence; both spawning a *bespoke* binary owned by
  one of them is.

When in doubt, omit — false positives are costlier than missed edges
since the user reviews proposals manually.

## Evidence grade

Annotate every proposal with one of:

- **`strong`** — symmetric artifact match (A produces X, B consumes X);
  a named wire protocol both sides declare; a reciprocated explicit
  mention.
- **`medium`** — one-sided evidence; shared format name without
  location; a shared external tool that's clearly one component's
  bespoke binary.
- **`weak`** — prose overlap; purpose similarity; shared data-format
  name without location. Weak edges are permitted but must declare
  weakness.

`evidence_fields` lists the surface paths that justify the edge (e.g.
`Alpha.surface.produces_files`, `Beta.surface.consumes_files`). May
be empty only when `evidence_grade: weak` AND `rationale` justifies
it explicitly.

## Output format

Write YAML to `{{PROPOSALS_OUTPUT_PATH}}` matching this shape:

```yaml
generated_at: <ISO-8601 UTC timestamp>
proposals:
  - kind: <kebab-case kind from the ontology above>
    lifecycle: <kebab-case lifecycle from the ontology above>
    participants: [<name>, <name>]    # see direction rules above
    evidence_grade: <strong | medium | weak>
    evidence_fields:
      - <e.g., "Alpha.surface.produces_files">
      - <e.g., "Beta.surface.consumes_files">
    rationale: |
      <one paragraph citing specific surface fields from the input>
```

Do NOT emit `schema_version` or `source_project_states` — those are
injected by the caller. Only propose edges between components that
appear in the input. Only use component names exactly as they appear
in the input — no paths, no aliases.

After writing the YAML, your final message should confirm the path
written. No other output is required.

## Input

The input below lists every catalogued component's surface record.

---
{{SURFACE_RECORDS_YAML}}
