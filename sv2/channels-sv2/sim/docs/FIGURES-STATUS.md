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
| `constraint_space.svg` (validity plane: direction-bias × responsiveness) | **SUPERSEDED FRAMING** — makes asymmetry/direction-bias the *validity axis* and the asymmetry-wall the admissibility edge; that's the pre-item-1 labeling. Geometry (binary wall bounding a region + floor orienting to gentlest interior point) is structurally fine | **RESTRUCTURE, not relabel** (see the SUPERSEDED-FRAMING comment in the SVG source): (1) relabel validity axis as dangerous-direction protection, (2) add a rate/regime dimension showing estimator-carries-sparse vs reluctance-carries-dense, (3) demote asymmetry from "the axis" to "the dense-regime mechanism." The current 2D plane can't hold the 3-part post-item-1 content. |

Other `docs/*.svg` are sim-generated data figures (tau_valley, tau_family, excess_lever,
floor_ribbon, steady_transient*, trajectory_plot) — measured output, not structural
framing, not affected by the item-1/rate-aware framing moves.

Companion (narrative) paper: the structural figures stay OUT regardless — wrong register
(abstract/geometric vs the companion's concrete/physical figures). The §10 deflation is
served by reusing the existing concrete figures, not adding abstract ones.
