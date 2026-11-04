# tracker-top

Pile top-of-pile (ToP) height optimisation for single-axis solar trackers, with a master pile
database, daily survey import and a 3D view. A Rust port of `SLSQP_ToP.py`, built to be fast,
shareable as a single executable, and reusable on other projects.

Two programs:

- **`tracker-top`**: the application. One executable that opens a web page in your browser
  (it runs on your computer, nothing goes online). Import files, read results, view the site in
  3D, export to Excel.
- **`top`**: the original command-line optimiser (CSV in, CSV out), unchanged.

## Getting it

- **Download**: every push builds Windows, macOS and Linux versions. Open the repository's
  *Actions* tab, choose the latest *build* run and download `tracker-top-windows-x64` (or the
  macOS/Linux one) from *Artifacts*. Tags such as `v0.2.0` also publish them as a release.
  Windows may warn that the program is unsigned: *More info* → *Run anyway*.
- **Build it yourself** (needs [Rust](https://rustup.rs)):

  ```
  cargo build --release
  ./target/release/tracker-top          # the application
  ./target/release/top --help           # the command-line optimiser
  ```

## Using the application

Run `tracker-top` (double-click on Windows). It creates `tracker-top.db` in the current folder
(use `--db <file>` to choose another; one database per project) and opens
<http://127.0.0.1:8770/>. Keep its window open while you work.

1. **Import → Master pile list**, once. Columns: pile ID, Easting, Northing, pile type, colour,
   and tracker, block, section, tracker type when available (without a Tracker column, the
   tracker is the pile ID without its last part: `P01.02.01.05` → `P01.02.01`). Excel or CSV.
2. **Import → LIDAR / design ground** (optional): elevations used for piles that have not been
   surveyed yet. A master list with a ground column loads it at the same time; `Z LIDAR_CF`
   (LIDAR with the planned cut/fill) is preferred over `Z LIDAR`, and the import report says
   which column was used.
3. **Import → Flood values** (optional, any time): see *Flood* below.
4. **Import → Daily survey report**, every day, or set a **watch folder** (below).
5. Trackers are **optimised automatically** after each import, as soon as every pile of a
   tracker has an elevation. Only trackers whose elevations or flood values changed are
   recomputed.
6. Read the **Results** (per tracker: status, profile chart, pile table), the **3D view**, the
   **History** of imports, and **Export Excel** (sheets *Piles*, *Trackers*, *Imports*,
   *Settings*) or CSV.

Tabs: *Overview* (progress by block), *Import*, *Results*, *3D view*, *History*, *Settings*
(reveal window, angle limit, survey-matching tolerances, watch folder).

### The master database

One SQLite file holds everything:

| Part | Rule |
|---|---|
| **Piles** (the core: ID, tracker, block, section, tracker type, Easting, Northing, pile type, colour) | Immutable. A later master list may only *add* new pile IDs; a file that changes an existing pile is refused whole. Enforced by database triggers, not just by the app. |
| **Imports** (every file, with its report) | Never deleted. A survey or LIDAR import can be *voided* (with a reason); its elevations stop being used and affected trackers are re-optimised. |
| **Elevations** (ground Z per pile per import) | Append-only. In use: the latest field survey of the pile, else its LIDAR / design value. |
| **Flood values** (per pile per import) | Append-only, voidable. In use: the latest one. Kept out of the core so they can be loaded or corrected later. |
| **Runs** (optimisation results) | Append-only. A tracker's current result is its latest run; results older than the current elevations or settings are flagged *out of date*. |

The same file imported twice is refused (it is recognised by content, not by name).

### Survey files

CSV, TXT, Excel (`.xlsx`, `.xls`, `.ods`), with or without a header row:

- Headers are recognised in English and French (`Point`/`Nom`, `Easting`/`X`/`Est`,
  `Northing`/`Y`/`Nord`, `Elevation`/`Z`/`Altitude`/`TN`, `Code`, `Date`…).
- Without a header, columns are read as Point, Easting, Northing, Elevation, Code (PENZD, the
  usual survey-controller export).
- `,` `;` tab or space separators; decimal comma or point; UTF-8 or Windows-1252.
- Survey date: the one entered on the Import page, else a Date column, else the file name
  (`2026-11-03`, `20261103`, `03.11.2026`…), else the import day.

Each point is matched to a pile by ID (case-insensitive), or by position (nearest pile within
0.5 m) when the ID is unknown. Points more than 1 m from their pile's design position are
rejected (wrong ID), more than 0.10 m are flagged, and an elevation that changed by more than
0.30 m since the previous survey is flagged. Control points and other non-pile points are
skipped and listed in the import report. A file whose elevation column is `Z LIDAR` is refused
as a survey. All tolerances are in Settings.

### Flood

Each pile can have a flood value (metres). Its top must stay above the flood water:

```
lowest allowed top = flood water level + clearance
flood water level  = ground + value        (Settings: "water depth above the ground", default)
                   = value                 (Settings: "water level, same datum as the ground")
```

The clearance (Settings, default 0) should include the drop from the top of pile to the lowest
module edge at maximum tilt, plus any freeboard. With no grading this raises the bottom of the
reveal window (e.g. [1.20, 1.60] becomes [1.50, 1.60] for 1.50 m of water); if the flood needs a
reveal above the maximum, the optimiser adds fill (tracker *Graded*), and if that would exceed
the cut/fill cap the tracker is *Non-compliant*.

Load the values with **Import → Flood values** (pile ID plus a column such as `Flood`,
`Flood depth (m)`, `Flood level`, `Inondation`, `PHE`), or as a `Flood` column of the master list.
A newer file replaces older values; a wrong one can be voided. Results show the flood level in
the tracker profile (dashed line, raised band), the pile table (▲ = raised), the 3D view (water
surface, *Flood* colouring) and the Excel export.

### Watch folder and Trimble Connect

`tracker-top` can import survey files automatically: in **Settings → Watch folder** (or with
`--watch <folder>`), give a folder on the computer. Every 30 s it imports each new CSV/TXT/Excel
file found there or in its subfolders as a field survey, then re-optimises. Files already
imported are skipped; files still being written wait for the next scan; results appear in
Settings and on the Overview.

To gather surveys from **Trimble Connect** with no programming, install **Trimble Connect Sync**
(or Trimble Connect for Windows) on the computer running `tracker-top`, sync the project folder
where the surveyors upload their daily reports, and point the watch folder at the local copy.

A direct connection to the Trimble Connect REST API (OAuth sign-in, polling a project folder)
is also possible and would remove the sync client, but needs an application registered with
Trimble; see *Not done yet*.

### 3D view

Ground mesh triangulated through the pile ground elevations (coloured by elevation or by
cut/fill), each pile drawn from its ground to its top, each tracker as a line through its pile
tops. Colour piles by tracker status, angle change, reveal, cut/fill, elevation source, flood,
or pile type in the site colours of the master list's Color column (one colour per type);
translucent flood water surface; adjustable vertical exaggeration; filter by block; go to a
tracker; click a pile for its details. Click legend entries to hide or show a category.

### Options

```
tracker-top [--db file] [--port 8770] [--host 127.0.0.1] [--watch folder]
            [--watch-interval 30] [--threads N] [--no-browser]
```

`--host 0.0.0.0` lets colleagues on the same network open it at `http://<your-computer>:8770/`.
There is no password: use it only on a trusted network.

## Command-line optimiser

```
./target/release/top optimize -i piles.csv -o results.csv [--block 1] [--max-angle 0.70] [--threads N]
./target/release/top optimize -i testdata/sample_block01_synthetic.csv -o /tmp/out.csv
```

Input columns: `Pile Name, Tracker, Easting, Northing, Z LIDAR` (required); `Block, Section, Pile Type` (optional).
Output columns follow the Python script (`NEW_ZG, CF, Optimized ToP, Final Reveal, Max_Angle, Angle_Diff, Comment`)
plus `Status` per tracker: `Compliant`, `Graded` (needs cut/fill), `NonCompliant`, `Skipped` (< 3 piles), `InvalidInput`.
Exit code 3 means some trackers could not be made compliant.

## How the optimisation works

It picks the ToP height of each pile so that the tracker follows the terrain as closely as
possible while keeping

- the angle change between two consecutive spans below the limit (default 0.70 deg; the NX Horizon XTR limit is 0.75 deg),
- the reveal inside its window (default 1.20 to 1.60 m around a 1.45 m nominal),
- cut/fill only where the first two cannot otherwise be met.

Per tracker, a convex QP in the ToP heights `y` and cut/fill `c`:

```
minimise   sum (y_i - c_i - z_i - reveal_nominal)^2 + lambda * sum c_i^2
subject to reveal_min <= y_i - z_i - c_i <= reveal_max,   |c_i| <= max_cut_fill
           |slope_i - slope_{i-1}| <= max_angle (rad)
           y_i >= min_top_i          (piles with a flood value: water level + clearance)
```

The slope-difference constraint is linear in `y` and always at least as strict as the true `atan2` angle
limit (atan is 1-Lipschitz), so results are never optimistic. True angles are recomputed and checked after
every solve. Cut/fill carries a large penalty, which replaces the old iterative +/-0.1 m grading loop.

The solver is a small dense OSQP-style ADMM (`qp.rs`) behind the `QpSolver` trait, so `clarabel` or another
solver can be dropped in. `top-core` has no external dependencies.

## Layout

```
crates/top-core   model, params, QP solver, optimiser, CSV I/O (library, std only)
crates/top-cli    `top` command-line tool (multi-threaded with std::thread::scope)
crates/top-db     master database (SQLite): import, matching, runs, Excel/CSV export
crates/top-app    `tracker-top`: local web server (axum) + web page (web/, built into the exe)
                  3D with three.js r170 and delaunator, bundled in web/vendor (MIT / ISC)
testdata/         small synthetic sample
```

## Verification done

- 44 automated tests: 21 optimiser (solver on analytic problems, flat/sloped/dipped terrain, randomised
  trackers, parallel == serial, flood minimum raising the window / forcing fill / unreachable), 2 CLI,
  21 database (file formats and header spellings, immutability triggers, duplicate files, ID and
  position matching, offset and jump checks, voiding, incremental re-runs, settings changes, flood
  layer in both modes, exports).
- Project master file `Piles_MUS_meanwidth_v1_2910_ZLIDARCF.xlsx` (36,913 piles, 2,792 trackers,
  41 blocks): imported and optimised in 0.8 s on `Z LIDAR_CF`: 2,791 compliant, 1 graded (max cut/fill
  0.03 m). The same piles on raw `Z LIDAR`: 64 graded (max 0.68 m), consistent with `Z LIDAR_CF`
  containing the planned earthworks. With a synthetic flood plane over the lowest area (472 piles, up
  to 2.2 m deep): 57 trackers re-optimised in 0.17 s, 22 piles raised, 14 trackers graded (fill up to
  0.60 m = 2.20 - 1.60), no top below its flood minimum.
- Optimiser, full run on 36,913 piles / 2,792 trackers (synthetic terrain over real geometry): 0.2 s on
  2 cores; independent recheck of angles, reveal and cut/fill in Python found 0 violations. Against SciPy
  SLSQP on 150 trackers needing no grading: ToP heights agree to a median of 0.05 mm, max 0.22 mm.
- Application, end to end on the sample (master list, headerless survey export, French `;`-CSV survey via
  the watch folder, LIDAR, voiding) and at 37,083 piles / 2,914 trackers: master import 0.24 s, LIDAR import
  + optimisation of every tracker 0.56 s, page data 0.3 s, Excel export 1.2 s, 3D view ready in < 1 s
  (4 cores, headless browser).

## Not done yet

- Direct Trimble Connect API connection (use Trimble Connect Sync + the watch folder meanwhile).
- Pile-type logic (`pile_name` in the Python script: HMP-31/SMP-31 above 6 % ground slope, minimum embedment).
- Piles already installed: hold their measured ToP fixed when re-optimising a tracker.
- Validation on real survey reports and real flood values (none available yet).
- Databases made by the 0.2.0 preview (schema 1) are refused: start a new one and import the master
  list again.
