# Tactical utility AI (P6 / TASK-0091)

Status: P6 implementation, opt-in. The legacy heuristic remains the default
until the P8 quality gate (TASK-0093) decides whether to enable the planner by
default.

This document describes `tools/combat-sim/src/ai.rs`. It does not change the
combat rules, the save schema, or the content format.

## Public entry points

```rust
let ai = Ai::prepare_with(&state, AiStrategy::Utility);
let preview = ai.preview_turn(&state);       // no mutation, no dice
ai.take_turn(&mut state, &mut rng, &mut log); // plans and executes
```

- `Ai::prepare` keeps `AiStrategy::LegacyHeuristic`; existing callers and save
  files are unchanged.
- `Ai::prepare_with(state, AiStrategy::Utility)` opts in.
- `Ai::preview_turn` returns a `TacticalPlan` with the behaviour profile, legal
  candidates, rejected candidates with reasons, and the deterministic order.
- `Ai::execute_tactical_plan` is the P7-facing execution seam: every candidate
  is re-validated against the current state before the engine call. It is also
  what `take_turn` uses internally.

## Pipeline

| Stage | What happens | Where |
|---|---|---|
| Behaviour / target profile | `BehaviourRole` (`guardian`, `escort`, `retreat`, `support`, `controller`, `skirmisher`, `striker`) plus `TargetProfile` (targeting, avoid conditions, downed-target policy), guard/flee zones, leash, range preference, heal/resource thresholds, opening list | `build_profile` |
| Candidate generation | Attacks, ability actions, multiattack, healing/support, defence, movement waypoints, configured scene interaction, wait/hold | `Planner::generate_*` |
| Legality filtering | Action economy, affordability, resource threshold, side/observability, range/reach, total cover | `action_precondition`, `action_legal_at_state` |
| Outcome valuation | Expected damage, effective healing, control, buff/defence, resource cost, cover delta, cooperation, objective, retreat/alert, consistency | `UtilityTerms`, `terms_for_outcome` |
| Utility scoring | Explicit weighted sum; deterministic tie-break | `UtilityTerms::score`, `CandidateEvaluation::sort_key` |
| Execution re-validation | Re-check the chosen candidate (and every fallback) against live state; try the next candidate on failure; fall back to the legacy heuristic if no utility candidate executes | `execute_candidates`, `validate_candidate` |

## Candidates

Covered:

- every resolved attack and ability action already planned by `Ai::prepare`;
- multiattack when every entry is legal at the current position;
- healing/self-healing, buffs and defence actions that carry an AI role;
- movement to tactical waypoints (guard zone, flee zone, objective zone,
  target/ally ring, cover neighbour, toward-target waypoint);
- retreat with disengage/dash when the profile's `flee_below_pct` triggers;
- scene objects whose `interact` block contains `toggle_trigger` or `activate`
  (treated as configured objective interaction);
- wait/hold when a guard profile is outside its leash and no better action is
  legal.

Explicitly excluded:

- `manual` / free-form scene interactions and interactions with no valued
  objective effect;
- actions whose entire effect list is unvalued by the current planner:
  `remove_condition`, `forced_move`, `use`, `activate`, `check`, `note`, and
  `toggle_trigger` as an action effect (the scene-object `toggle_trigger` form
  is supported as an objective interaction);
- multiattack with a movement approach (multiattack is only considered from
  the current position);
- joint action selection across creatures. Each creature scores its own
  candidates and only reads ally positions/goals; `Ai::take_turn(&self)` stays
  immutable and rayon-safe.

Mixed effects are allowed: valued parts are scored and unvalued parts are
ignored. Actions with no valued part are rejected with a recorded reason.

## Utility terms and default weights

Weights live in `UtilityWeights::DEFAULT` inside `ai.rs`. This is a P6
temporary approach because `content/schema.rs` and `ai-templates.yaml` are
outside this task. A later task can move the same structure into configuration
without changing the planner pipeline.

| Term | Default weight | Meaning |
|---|---:|---|
| `expected_damage` | 1.0 | expected damage after hit/crit/save probabilities, capped at remaining HP |
| `ally_risk_reduction` | 1.4 | buff/defence/control value that reduces risk for the actor's side |
| `healing` | 1.2 | effective healing; downed targets get a `2.5x` rescue multiplier |
| `control` | 2.2 | condition value scaled by the target's threat |
| `objective_progress` | 6.0 | configured interaction, hold point, retreat/flee-zone progress, escort/cover movement |
| `resource_opportunity_cost` | 1.5 | cost of spending a limited resource, higher when it is the last point |
| `cover_positioning` | 2.0 | cover gained relative to the current position |
| `cooperation` | 1.2 | support for wounded/downed allies and ally proximity |
| `retreat_alert_cost` | 1.5 | cost of retreating while adjacent to enemies or leaving the guard zone |
| `behaviour_consistency` | 1.0 | match with targeting/range/opening/guard/flee policy and avoid conditions |
| `movement_cost` | 0.05 | feet travelled, expressed in 5-ft squares |

Score:

```text
+ damage + risk + healing + control + objective + cover + cooperation + consistency
- resource cost - retreat alert cost - movement cost
```

All terms are exposed as public fields on `UtilityTerms`; `UtilityTerms::score`
takes a `UtilityWeights`, so a P8 benchmark can compare weight sets without
changing the engine.

## Legality and re-validation

- Candidate filtering uses the same public `Space`/`SpaceView` queries as the
  engine for distance, reach, cover and route cost, plus the same affordability
  and resource-threshold rules as the legacy AI.
- Attack outcome estimates call `engine::forecast_attack` /
  `engine::simple_attack_forecast`; the planner never rolls dice.
- Every executed action goes through `engine::attack_at`,
  `engine::use_planned_ability`, `engine::move_to_at` or `engine::interact`, so
  the engine re-checks action economy, targets, range, cover, line of sight,
  resources and triggers.
- `execute_tactical_plan` re-validates every candidate immediately before
  execution. A target that becomes dormant/dead, a route that closes, a
  resource that is spent, or an engine rejection causes the next candidate to
  be tried. If none executes, `take_turn` falls back to the legacy heuristic.

Known limit: line-of-sight/height details and a few engine-only rejection
paths are authoritative in `engine.rs`; the planner can still propose a
candidate that the final engine call rejects. The re-validation path and the
fallback are tested. Moving the shared legality check into a dedicated engine
API would require editing a forbidden P6 path and is left to a later task.

## Observable information

The planner filters:

- dormant creatures;
- dead creatures;
- living targets with `hp <= 0` unless the profile explicitly allows
  `attack_downed` or a `heal_downed` rescue;
- targets whose active condition definition has the generic `Unaware` flag.

Cover, threat, objective urgency and retreat checks only use the resulting
observable enemy set. Allies are read for cooperation, but hidden enemy
positions are not used to score movement.

Known limit: `EncounterState` currently has no general "hidden/unrevealed
token" flag. The 2024 `hidden` condition is a combat modifier, not a visibility
flag, so it is not treated as unrevealed. This needs a belief-state/visibility
task; P6 documents the limit instead of changing `state.rs` or the schema.

## Movement pruning

`MAX_MOVEMENT_CANDIDATES` is `8`. The planner builds tactical anchors, not the
full reachable set:

- current position, guard/flee/objective zones;
- target and ally rings;
- cover neighbours of the actor;
- one toward-target waypoint;
- zone positions for objective objects (first deterministic cells only).

Every anchor must have a legal `route_cost` within the remaining speed budget.
The test on a 15x15 open grid asserts the distinct waypoint count stays at or
below the cap.

## Determinism and parallelism

- Planning consumes no dice and no RNG.
- Candidates are sorted by `f64::total_cmp` on score and then by a stable
  candidate key (action id, role, target, position, object id).
- `Ai::take_turn(&self)` only reads `Ai`; no shared mutable state is introduced.
  The same state and seed produce the same turn.
- `AiTargeting::Random` is deterministic in the utility planner (it has no
  per-target preference); the legacy heuristic keeps its existing random
  behaviour.

## Quantitative comparison

The synthetic comparison is in
`tools/combat-sim/tests/ai_tactics.rs::quantitative_comparison_reports_improvement_and_parity`.
It uses the same seeded RNG for each strategy.

Observed on the task branch:

```text
P6 comparison: objective legacy=false utility=true; melee damage legacy=3 utility=3
```

- Improvement: the utility planner uses the configured objective interaction;
  the legacy heuristic does not complete the objective in the same synthetic
  turn.
- Non-improvement: the baseline melee turn deals the same 3 damage for both
  strategies. This is not evidence of a general win-rate improvement; P8 must
  run the broader benchmark, including non-improvements and timeouts.

## Tests

`tools/combat-sim/tests/ai_tactics.rs` covers:

| AC | Test |
|---|---|
| AC-1 | `preview_separates_profile_candidates_and_legality`, `utility_terms_have_explicit_weights` |
| AC-2 | `preview_separates_profile_candidates_and_legality`, `escort_profile_holds_with_ally_and_supports`, `retreat_uses_disengage_and_flee_zone`, `stop_objective_interacts_with_configured_object` |
| AC-3 | `utility_terms_have_explicit_weights`, `preview_separates_profile_candidates_and_legality` |
| AC-4 | `execution_revalidates_before_acting` |
| AC-5 | `hold_point_does_not_chase_beyond_leash`, `escort_profile_holds_with_ally_and_supports`, `retreat_uses_disengage_and_flee_zone`, `heal_downed_ally_scores_rescue`, `resource_conservation_keeps_the_limited_resource`, `cover_candidate_moves_into_cover`, `stop_objective_interacts_with_configured_object` |
| AC-6 | `quantitative_comparison_reports_improvement_and_parity` |
| AC-7 | `candidate_generation_skips_dormant_dead_and_unaware_targets` |
| AC-8 | `fixed_seed_and_preview_order_are_deterministic`, `utility_strategy_completes_a_synthetic_encounter`, fmt/clippy/CI |
| AC-9 | `movement_candidates_are_capped_on_a_large_grid` |

Run:

```bash
CARGO_BUILD_JOBS=2 cargo test --manifest-path tools/combat-sim/Cargo.toml --test ai_tactics
CARGO_BUILD_JOBS=2 cargo test --manifest-path tools/combat-sim/Cargo.toml
CARGO_BUILD_JOBS=2 cargo clippy --manifest-path tools/combat-sim/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tools/combat-sim/Cargo.toml --check
```

## Non-goals

- Multi-turn search, adversarial responses, or MCTS (P7/TASK-0092).
- Joint party planning or cross-creature mutable state.
- Data-driven weight configuration (deferred; P6 keeps explicit constants).
- Hidden-token belief state or DM-only information access.
- Changing the save schema, the engine legality pipeline, or the CLI.
