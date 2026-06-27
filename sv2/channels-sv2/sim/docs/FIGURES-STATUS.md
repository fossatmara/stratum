# Figure status against the closed vardiff theory

Status of the **structural** figures (the abstract algorithm-space diagrams, as
opposed to the concrete physical figures) against the closed single-sensor theory,
recorded so a stale-but-clean-looking figure is not reused as finished. Supersession
framing throughout: a flagged figure is **correct for the theory as drawn**, with its
**framing superseded** — not "wrong."

The closed theory these are checked against:
- **Validity = dangerous-direction protection** (suppress self-deepening over-difficulty
  fires during an escape). Eager-ease/asymmetry is **one mechanism** for it, not the axis.
- **Regime split**: at SPARSE rate the **estimator** provides the protection entirely
  (sets fire direction + slow window doesn't present up-spikes); the sparse boundary is
  PoissonCI (symmetric, a **trigger**, no protective content). At DENSE rate
  **SignPersistenceCusum's reluctant-tighten** provides it. (stratum commits: item-1
  resolved 9518f3bf; boundary misattribution corrected a6ccec7f; rigs `eager-ease-*`,
  `cellA-mechanism`, `which-boundary`.)
- **Rate-awareness lives entirely in the ordering/wobble half** — no admissibility
  content; it's policy, not safety. (commits a83be52a, ac21e775, 43fa3216.)

| figure | status | action before technical-paper reuse |
| --- | --- | --- |
| `tau_tradeoff.svg` (evaluation plane: over-difficulty × wobble) | **CONSISTENT** — gate (over-difficulty, U-shaped, walls both ends = admissible island) / frontier-ordering (wobble) / champion = gentlest admissible all survived the arc; already labels dangerous vs safe axes | reusable; **additive** extension only — draw rate-awareness as wobble-half motion that never reaches the gate wall (makes the "policy not admissibility" result visible) |
| `dangerous_direction_protection.svg` (§6.1 mechanism map: which mechanism protects, per regime) | **BUILT + CONSISTENT** (f72f9d07-era) — the §6.1 figure. Two-panel regime split at spm6; SPARSE = estimator sole-necessary (boundary regime-locked-out); DENSE = boundary active-but-substitutable. Embedded in §6.1. Structural map (what-protects-where), deliberately distinct from tau_tradeoff's selection plane (where-champion-sits); makes no per-mechanism-curve claim (mechanism detail stays in §6.1 prose). Asymmetric, NOT symmetric "each covers a regime the other can't." | none — current. Verified by `eager-ease-strength.rs`. |
| `constraint_space.svg` (validity plane: direction-bias × responsiveness) | **SUPERSEDED + REPLACED** — makes asymmetry/direction-bias the *validity axis* (pre-item-1 labeling). The RESTRUCTURE it needed is now DONE as a *new* figure (`dangerous_direction_protection.svg`), not a redraw of this one: the post-item-1 content is *mechanism* (what-protects-where, regime-split), not a corrected *region* plane — the region job is tau_tradeoff's (the admissible island), and this figure's region-with-asymmetry-as-axis framing was itself the staleness. So `constraint_space.svg` is **retired** (kept on disk with its SUPERSEDED-FRAMING comment as the record; do NOT embed or reuse). | retired — superseded by `dangerous_direction_protection.svg` for the mechanism content and `tau_tradeoff.svg` for the region content. |

Other `docs/*.svg` are sim-generated data figures (tau_valley, tau_family, excess_lever,
floor_ribbon, steady_transient*, trajectory_plot) — measured output, not structural
framing, not affected by the item-1/rate-aware framing moves.

Companion (narrative) paper: the structural figures stay OUT regardless — wrong register
(abstract/geometric vs the companion's concrete/physical figures). The §10 deflation is
served by reusing the existing concrete figures, not adding abstract ones.
