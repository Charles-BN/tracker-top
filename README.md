# tracker-top

Pile top-of-pile (ToP) height optimisation for single-axis solar trackers. A Rust port of
`SLSQP_ToP.py`, built to be fast, shareable as a single executable, and reusable on other projects.

Given the LIDAR ground elevation at every pile, it picks the ToP height of each pile so that the
tracker follows the terrain as closely as possible while keeping

- the angle change between two consecutive spans below the limit (default 0.70 deg; the NX Horizon XTR limit is 0.75 deg),
- the reveal inside its window (default 1.20 to 1.60 m around a 1.45 m nominal),
- cut/fill only where the first two cannot otherwise be met.

## Usage

```
cargo build --release
./target/release/top optimize -i piles.csv -o results.csv [--block 1] [--max-angle 0.70] [--threads N]
./target/release/top --help
```

Try it on the bundled sample (synthetic terrain over real pile positions):

```
./target/release/top optimize -i testdata/sample_block01_synthetic.csv -o /tmp/out.csv
```

Input columns: `Pile Name, Tracker, Easting, Northing, Z LIDAR` (required); `Block, Section, Pile Type` (optional).
Output columns follow the Python script (`NEW_ZG, CF, Optimized ToP, Final Reveal, Max_Angle, Angle_Diff, Comment`)
plus `Status` per tracker: `Compliant`, `Graded` (needs cut/fill), `NonCompliant`, `Skipped` (< 3 piles), `InvalidInput`.
Exit code 3 means some trackers could not be made compliant.

## How it works

Per tracker, a convex QP in the ToP heights `y` and cut/fill `c`:

```
minimise   sum (y_i - c_i - z_i - reveal_nominal)^2 + lambda * sum c_i^2
subject to reveal_min <= y_i - z_i - c_i <= reveal_max,   |c_i| <= max_cut_fill
           |slope_i - slope_{i-1}| <= max_angle (rad)
```

The slope-difference constraint is linear in `y` and always at least as strict as the true `atan2` angle
limit (atan is 1-Lipschitz), so results are never optimistic. True angles are recomputed and checked after
every solve. Cut/fill carries a large penalty, which replaces the old iterative +/-0.1 m grading loop.

The solver is a small dense OSQP-style ADMM (`qp.rs`) behind the `QpSolver` trait, so `clarabel` or another
solver can be dropped in. The crates have no external dependencies.

## Layout

```
crates/top-core   model, params, QP solver, optimiser, CSV I/O (library, std only)
crates/top-cli    `top` command-line tool (multi-threaded with std::thread::scope)
testdata/         small synthetic sample
```

## Verification done

- 17 unit tests (solver on analytic problems, flat/sloped/dipped terrain, randomised trackers, parallel == serial) and 2 CLI tests.
- Full run on 36,913 piles / 2,792 trackers (synthetic terrain over real geometry): 0.2 s on 2 cores; independent
  recheck of angles, reveal and cut/fill in Python found 0 violations.
- Against SciPy SLSQP (original formulation) on 150 trackers needing no grading: ToP heights agree to a median
  of 0.05 mm, max 0.22 mm.

## Not done yet

- Excel input/output (`calamine`, `rust_xlsxwriter`): needs crates.io, which was not reachable when this was started.
  Until then, export the sheet to CSV.
- Pile-type logic (`pile_name` in the Python script: HMP-31/SMP-31 above 6 % ground slope, minimum embedment).
- 3D viewer (Bevy + egui) and the 2D profile viewer.
- Validation on real LIDAR data: `Piles_MUS_meanwidth_v1_0910.xlsx` has no `Z LIDAR` column.
