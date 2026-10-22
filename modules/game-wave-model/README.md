# Isolated game wave model

A dependency-free Rust research crate for Akula. It is not linked to the main
`aquilum-app` workspace and does not load, execute, or redistribute Flash
files or third-party game code.

## Run

```sh
cargo test --manifest-path modules/game-wave-model/Cargo.toml
```

The code deliberately operates on typed `Wave`, `Group`, and `Spawn`
records. Parsing external XML/SWF/FLA is not implemented. The importer must
first validate and normalize untrusted input; `Scheduler` performs bounded
allocation at construction and contains no unsafe code.

## Scope and fidelity

- One `tick()` is one logical game update, **not** one millisecond.
- The first item in each batch appears immediately; subsequent items use a strict `>` frequency comparison. Pauses use a strict `>` threshold.
- Groups advance when their spawn slots are issued, not when enemies die.
- Wave completion waits for spawned objects unless a positive timeout expires
  after all groups are issued.
- Transition between waves is modelled as a 150-update gap.
- The host reports object removal via `mark_gone(id)`. Reusing a cyclic spawn slot does not erase earlier objects from the host's live set.
- Global configuration and live-object budgets limit resource use; exceeding the live-object limit halts ticks with an explicit error.
- A provided index chooser permits reproducible shuffle tests without claiming
  parity with Flash Player's `Math.random()`.
- Optional cycling preserves the source-like rest/count mechanics but is not
  yet verified against a running reference SWF.
- Renderer, physics, audio, saved games, networking, and portal APIs are **out
  of scope**. This crate alone cannot run a Flash game.

## Evidence and limitations

The design was informed by reading `Wave.as`, `WaveController.as`,
`waves.xml`, and `enemies.xml` in
[TylerGlaiel/GlaielGamesOldFlashSources/tetraform](https://github.com/TylerGlaiel/GlaielGamesOldFlashSources/tree/master/tetraform).
No files, data or ActionScript from that repository were copied into this
crate. The third-party game's CC BY-NC-SA terms require separate license review
before redistributing original game assets.

This is an **original testable approximation**. Static review uncovered and fixed two model divergences (initial batch timing and live cyclic objects); 15 targeted Rust tests passed on GitHub Actions run 39796612720 (14 scheduler tests plus one source-derived trace test containing eight synthetic cases). This is not proof of parity with Flash. In particular: random
sequences, exact transition timing, removal/respawn identity, empty-group
handling and cycling must be validated with original runtime traces before
claiming behavioral equivalence. The unit tests test internal contracts, not
runtime parity.

## CI impact

Only three files under `modules/game-wave-model/` are added.
`aquilum-app/Cargo.toml` and its lockfile remain unchanged; no new
`.github/workflows` file, dependency or mandatory status is introduced.
Under the existing scope classifier, a module-only change is `mode=none`,
so normal PR checks will **not** run these Rust tests. This is intentional
to avoid increasing general CI fan-out. A targeted test on a Rust-enabled
machine (command above) is a separate acceptance requirement.

## Follow-up acceptance

1. Completed: `cargo test --manifest-path modules/game-wave-model/Cargo.toml` on GitHub Actions run 39796612720 (15 passed, 0 failed). The temporary workflow was removed afterwards.
2. Completed for source-derived synthetic cases: pause, spawn, cycle and next-wave transitions. Actual Flash Player reference traces remain to be collected.
3. Investigate precise live-object slot semantics and scripted resource joins.
4. Add a narrow, shared test lane only after the baseline is measured and
   there is explicit approval for its CI cost.
