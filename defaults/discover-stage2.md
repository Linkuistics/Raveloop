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

### Role hints are priors, not verdicts

Each surface record may carry an optional `interaction_role_hints`
list — self-declared labels a component's own prose assigns itself
(e.g. `generator`, `orchestrator`, `server`, `client`). Treat these
as **priors only**: they can raise your prior that a particular
decision-tree branch applies to a pair, but an edge still requires
cross-referenced surface-field evidence (overlapping paths, matching
endpoints, named tools, etc.). Never propose an edge solely because
one side's hint suggests it.

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

## Output — invoke the CLI once per edge

Do NOT write a YAML file. For each edge you propose, invoke this shell
command via the `Bash` tool:

```bash
ravel-lite state discover-proposals add-proposal \
  --config "{{CONFIG_ROOT}}" \
  --kind <kebab-case kind from the ontology above> \
  --lifecycle <kebab-case lifecycle from the ontology above> \
  --participant <name1> \
  --participant <name2> \
  --evidence-grade <strong|medium|weak> \
  --evidence-field "<e.g., Alpha.surface.produces_files>" \
  --evidence-field "<e.g., Beta.surface.consumes_files>" \
  --rationale "<one paragraph citing specific surface fields from the input>"
```

Rules:

- **One invocation per edge.** A pair with two `(kind, lifecycle)` tuples
  (§3.5) is two invocations.
- **Participant order matters for directed kinds.** The first
  `--participant` is the canonical-order "from" component and the
  second is the "to" component (e.g. `generates`: first produces,
  second consumes). For symmetric kinds the CLI canonicalises to
  alphabetical order internally, so either order is accepted.
- **Only catalogued components.** Use component names exactly as they
  appear in the input — no paths, no aliases. The CLI rejects unknown
  names with a list of the valid ones.
- **Repeat `--evidence-field` for multiple fields.** Omit the flag
  entirely only when `--evidence-grade weak` (the CLI will reject
  empty evidence on `strong`/`medium`).

If an invocation returns a non-zero exit, read the stderr — it will
cite the invalid argument and list the valid vocabulary or catalog
entries. Correct the argument and retry. When every edge has been
proposed, exit. No summary message is required.

## Input

The input below lists every catalogued component's surface record.

---
{{SURFACE_RECORDS_YAML}}
