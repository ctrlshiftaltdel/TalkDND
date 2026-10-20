# pdf2md review lessons

Notes from the first Grok Build review pilot (Keys from the Golden Vault) and the early Player's Handbook 2024 runs. No book text is quoted here—only scores and process findings.

## Pilot snapshot (KGV)

| Metric | Value |
|--------|------:|
| Pages reviewed | 22 |
| Accepted | 18 (82%) |
| Rejected | 4 (later retries: 3 reject + 1 max-turns error) |
| Mean time / accepted page | ~215 s |
| Inferred markers on accepted pages | 1 |

Accepted pages that had a usable PDF text layer moved roughly **recall 0.73 → 0.93** and **unknown words/1k 141 → 6**. Ten accepted pages had no usable text layer (scans); those are judged mainly on unknown/1k and structural flags.

## Reject / error patterns

1. **Stat-block soft regressions (majority of rejects).** After the line-end hyphenation fix (TASK-0017), remaining KGV rejects were all `statblock` pages where recall fell by about **0.01–0.05** while unknown/1k improved and word count stayed **≥ 93%** of the before score. The old gate rejected any recall drop, so cleaned-up ability lines and reformatted blocks looked like regressions.
2. **Real deletions.** Early PHB run: one page dropped to **~67%** of prior word count with a recall drop **> 0.10**. That must stay rejected.
3. **Max-turns errors.** Grok Build hit `--max-turns` (default 40) on dense pages (~500–600 s then exit 1). A single retry with a shorter, no-search prompt recovers many of these.
4. **Reference lookup.** Web search is off by design (privacy). Listing SRD file paths in the prompt was not enough—the agent rarely opened them. Injecting short, name-matched local snippets works better.
5. **Inference.** Owner rule: when still unsure after the page image and local SRD, infer from context and mark `<!-- inferred: reason -->`.

## Gate thresholds (TASK-0018)

| Rule | Threshold |
|------|-----------|
| Hard reject: unknown/1k rises | any increase |
| Hard reject: deletion | words < **50%** of before |
| Soft recall allowance | drop ≤ **0.05** |
| Soft word floor | words ≥ **90%** of before |
| Soft also requires | unknown/1k **decreases**, badness **does not worsen**, and (`statblock` flag **or** structure markers such as Armor Class / Hit Points / STR table) |
| Default Grok max-turns | **60** (retry **30** with short prompt) |
| Injected local snippet budget | **4000** characters |

`pdf2md review --rejudge` re-scores `fixes/rejected/` with the current gate and promotes soft improvements without calling an agent.

## Operational notes

- Review skips pages that already have an accepted fix; safe to stop and resume a background job after merging gate changes.
- `apply` waits for the pdf2md worker lock before rebuilding `<book>.md`.
- Corrected book text stays under local `.md/` trees and must never enter git, Issues, PRs or CI logs.
