# Memory Style Rules

All `memory.md` entries in a backlog plan follow these rules. Both the
reflect phase (incremental per-session) and the dream phase (periodic
global rewrite) produce entries in this style.

## Three principles

**1. Assertion register, not narrative register.** State what is true,
not how we learned it. Session numbers, dates, "previously", "initially",
"after the fix" — none of these appear in memory. If the fact is "X does
not work because Y", write that; do not write the story of discovering
it.

**2. One fact per entry.** If an entry covers two distinct facts, split
them. Multi-fact entries accrete prose because joining facts requires
connective narrative, which is itself the verbosity problem.

**3. Cross-reference, don't re-explain.** If entry B depends on entry A,
reference by heading. Headings are addresses. Do not restate A inside B.

## Concrete guidance

| Don't | Do |
|---|---|
| "Session 29 lifted macOS IoU F1 from 6% to 26.4%; Session 30 fixed Linux..." | "Center-distance matching replaces IoU threshold when bounding boxes include padding." |
| Parenthetical status headings: "X (Sessions 36/37/38, retired by 41, falsified 42)" | Short subject-predicate headings, ~8 words max |
| Keep retired/falsified hypotheses as entries | Convert to "X does not work because Y" (live fact) or delete (git remembers) |
| Inline measurements as narrative decoration | Keep load-bearing numbers; drop numbers that were evidence at the time |
| Re-explain project background in every entry | First sentence is the claim, not the context |

Additional guidance:

- Headings: short subject-predicate form, ~8 words max
- Drop session numbers, dates, "previously"/"initially"/"after the fix"
- Drop retired/falsified hypotheses — convert to "X does not work because Y"
  (live fact) or delete (git remembers)
- Keep load-bearing numbers; drop numbers that were evidence at the time
- First sentence is the claim, not the context

## Example transform

Before (~100 words):

> ### Apple Vision dense-monospace gap is **Apple-Vision-specific**, not OCR-task-bound (Sessions 36/37/38/40, **retired as a default-path bound by Session 41**, segmentation-rescue falsified Session 42)
> Apple Vision OCR cannot read dense monospace terminal scrollback on any platform, and Session 38's `--preprocess` spike falsified the segmentation-fix hypothesis bit-exactly *for Apple Vision*: 2x/4x `CILanczosScaleTransform` produced terminal F1 deltas of +0.18pp macOS / +0.06pp Linux against a ≥10pp ship threshold. Vision did *segment* more rows under upscale...

After (~45 words):

> ### Apple Vision cannot read dense monospace terminal scrollback
> Recognition-bound, not segmentation-bound: upscaling via `CILanczosScaleTransform` improves row segmentation but yields <1pp F1 against a ≥10pp target. Applies on all platforms. Use EasyOCR for terminal content.

Same facts, less than half the words.

## Lossless vs lossy

- **Reflect** may prune aggressively — it is the lossy-pruning phase.
  Remove outdated or redundant facts.
- **Dream** is **strictly lossless** — preserve every live fact, only
  rewrite prose. Do not delete entries unless they are pure duplicates
  of others.
