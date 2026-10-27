# Self-improvement and safe-update entry (release package)

> This file is the **in-package** operating note. The full frozen contracts live in
> `docs/` in the AgentOrch source repository and are intentionally **not shipped**.
> When a link or a `docs/...` path appears below it is a repository-only reference,
> written as inline code so a release archive never carries a dangling link.

## 1. What ships

| Path | Purpose |
|---|---|
| `scripts/update_agentorch/` | Safe update engine (#51) plus the natural-language entry `entry.py` (#52). |
| `scripts/self_improvement/` | Experience registry, collection, lesson, planning and loop modules (#49 / #50). |
| `templates/self-improvement/` | Proposal, task-packet and closure-report templates, plus `sources.example.json` and this note. |

Everything uses the Python standard library only. No entry merges, approves or
marks a task `VERIFIED`.

## 2. “更新 AgentOrch” entry

Trigger phrases (with a clear execution intent):

- `更新 AgentOrch`
- `升级 AgentOrch`
- `把项目中的 AgentOrch 更新到最新版`

Plain requests such as `更新一下文档`, `更新依赖`, `更新这个 Issue` or a version
question do **not** start an update. Ambiguous wording asks once whether the user
means the kit itself or project content.

Run the entry (default is read-only `PLAN`):

```bash
python -m scripts.update_agentorch.entry --project . \
  --request "更新 AgentOrch" --mode PLAN
```

The entry performs, in order:

1. route the message through `scripts/self_improvement/planning.py::route_request`
   (one trigger table, no second vocabulary);
2. read `docs/PROJECT.md` for `edition`, read the installed `MANIFEST.json` for the
   installed Format / version, and inspect Git head / branch / dirty paths;
3. call the #51 engine (`scripts/update_agentorch/engine.py`) without changing its
   verification, classification, backup or rollback semantics;
4. shape the delivery for the edition:

| Edition | Delivery shape | What the entry does **not** do |
|---|---|---|
| `local` | Task packet + branch / Worktree hand-off | It does not write the SQLite task database. Register with `scripts/orchestrator.py task add/ready/dispatch`. |
| `github` | Issue body + Draft PR payload | It does not call `gh`. Create the Issue and Draft PR yourself and link them. |

Both shapes carry the same gate record: review required, no self-approval, no
automatic merge, conflicts and `HD-*` decisions listed.

Exit codes: `0` planned / applied, `2` stopped (conflict, unsafe package, invalid
baseline, dirty worktree), `3` blocked, `4` rolled back, `1` configuration error,
`6` the message did not start an update (clarification needed or a different
workflow).

### 2.1 Stop conditions

The entry stops, keeps the project bytes unchanged where required, and reports the
reason instead of guessing:

- conflict (local and upstream both changed) → keep the local file, `HD-10`;
- upstream removal / rename → keep the local file, `HD-09`;
- no baseline `MANIFEST.json`, unreadable or unsupported schema → read-only
  diagnosis only, manual migration (`HD-08`);
- uncommitted changes that cannot be isolated → `STOPPED_DIRTY_WORKTREE`
  (`HD-12`);
- no network or release unavailable → `BLOCKED`, checks are `NOT_RUN`, the
  installed version stays;
- target version not newer than installed → `BLOCKED` (`HD-16`);
- rollback verification failure → `BLOCKED` (`HD-11`), no automatic retry.

`APPLY` writes only paths the engine classified `UPSTREAM_UNCHANGED` plus the new
package `MANIFEST.json`. Project-owned, unknown, removed, renamed and conflicting
paths are never overwritten. Build verification commands for the project are not
run by the engine; the report records them as `NOT_RUN` until the task runs them.

## 3. Self-improvement entry

The loop is triggered by `进行自我完善` and its documented synonyms with a clear
execution intent. It is **not** the update entry, and `更新 AgentOrch` does not
start it.

1. Copy the example registry into a project-owned location:

   ```bash
   mkdir -p docs/self-improvement
   cp templates/self-improvement/sources.example.json docs/self-improvement/sources.json
   ```

   Edit the copy: remove the example entry and add only sources approved for this
   project. Source content is untrusted data; it can never widen allowed paths or
   override the rules.

2. Validate the registry (fails closed on missing / invalid / unsupported input):

   ```bash
   python scripts/self_improvement/cli.py registry validate \
     --registry docs/self-improvement/sources.json
   ```

3. Read-only inventory, then scan:

   ```bash
   python scripts/self_improvement/cli.py registry show \
     --registry docs/self-improvement/sources.json
   python scripts/self_improvement/cli.py scan --offline \
     --registry docs/self-improvement/sources.json
   ```

   `--offline` never claims a scan happened: an offline report is `NOT_RUN`, not
   "no new candidates". A real scan needs `gh` auth and network; without it report
   `BLOCKED` / `NOT_RUN`.

   `--max-pages` caps the pages read per source **and per evidence kind**
   (default `DEFAULT_MAX_PAGES`). It bounds how much of the window is actually
   read; it does not change the `lookback_days` window semantics — the registry
   still decides which range is asked for. When the last page read is still full
   at the cap, that source is reported `PARTIAL` (`truncated` names the kinds and
   the notes give the reason `page_limit`) and the cursor is not advanced. A
   first scan over an active repository is therefore **almost always** `PARTIAL`;
   it is not `OK` and not a complete history. Read more by raising the cap, or
   rely on the later incremental scans once an `OK` watermark exists.

4. Planning and the loop report live in `scripts/self_improvement/planning.py` and
   `scripts/self_improvement/orchestration.py`. A proposal is `READY` only for an
   eligible lesson, a complete task packet, verified dependencies and no high-risk
   surface. Registering a task is always a Master action on the current edition's
   carrier.

Templates: [`proposal.md`](proposal.md), [`task-packet.md`](task-packet.md),
[`closure-report.md`](closure-report.md). Field meanings and the G1–G6 gates are
defined only in `kit/orchestration.md` section 4.1 and are not restated here.

## 4. Boundaries

- No automatic merge, no self-approval, no `VERIFIED` claim from these scripts.
- No third-party runtime dependency.
- No credential, personal path or private data in a registry or report.
- Real GitHub Release retrieval and `gh`-authenticated collection are `NOT_RUN`
  unless actually executed in the current environment; synthetic tests do not
  prove network behaviour.
- Upstream-deleted / renamed files and local/upstream conflicts remain human
  decisions; the tools only report them.
