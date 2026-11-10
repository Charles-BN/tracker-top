# tracker-top

Pile top-of-pile (ToP) height optimisation for single-axis solar trackers, with a master pile
database, daily survey import and a 3D view. A Rust port of `SLSQP_ToP.py`, built to be fast,
shareable as a single executable, and reusable on other projects.

Three ways to run it, one code base:

- **Browser version**: a web page (Edge, Chrome). The whole program runs inside the page
  (WebAssembly) and the database is stored in that browser, on that computer; nothing is sent to a
  server and nothing is installed. For computers that may not run programs.
- **`tracker-top`**: the desktop application. One executable that opens the same page in your
  browser, with the database in a file next to it.
- **`top`**: the original command-line optimiser (CSV in, CSV out), unchanged.

The desktop program and the browser version can share **one database** through a Google Drive
folder (or OneDrive, a network drive), with no server: see *Shared database*.

How the code works, for someone who codes a little but is not a Rust expert:
[docs/HOW-THE-CODE-WORKS.md](docs/HOW-THE-CODE-WORKS.md).

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

## Browser version

Open the site (see *Publishing the browser version*), then use it exactly like the desktop
application below. Differences:

- The database lives in the browser's storage for that site, on that computer, unless it is
  shared (see *Shared database*: then every change is also written to the shared folder). Without
  sharing, **Settings → Database → Download a copy** regularly: clearing the browser's site data, or
  using another browser or computer, starts from an empty database. *Open a database file* loads a
  copy (from the browser version or the desktop program, which share the same file format).
- The watch folder (Settings) is chosen with the browser's folder picker (Edge, Chrome) and works
  while the page is open.
- Calculations run on one processor core: importing and optimising the 36,913-pile master list
  takes about 3 s.

### Publishing the browser version

The browser version is a folder of static files (`scripts/build-web.sh` builds it into
`target/web-dist/`; it needs `rustup target add wasm32-unknown-unknown` and
`cargo install wasm-bindgen-cli --version 0.2.130 --locked`). Any web server can host it; it does
not work opened from disk (`file://`).

With GitHub Pages: in the repository, *Settings → Pages → Source: GitHub Actions* (on a private
repository this needs a paid GitHub plan). The *browser version* workflow then publishes it at
`https://<account>.github.io/tracker-top/` on every push to `main`, and keeps the built folder as a
downloadable artifact for hosting it elsewhere (e.g. a company web server).

## Using the application

Run `tracker-top` (double-click on Windows). It creates `tracker-top.db` in the current folder
(use `--db <file>` to choose another; one database per project) and opens
<http://localhost:8770/>. Keep its window open while you work.

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
   *Settings*), **Custom…** (your choice of columns, blocks and sheets, see *Custom export*), CSV,
   or **ArcGIS** (GeoPackage, see *ArcGIS* below).

Tabs: *Overview* (counts, **average reveal**, progress by block with each block's average reveal
and installers' file), *Import*, *Results*, **Map** (2D plan, see below), *3D view*, *History*,
*Settings* (reveal window, angle limit, rounding, survey-matching tolerances, installers' files,
watch folder, database, shared database).

**Ground elevations**: for every pile the *Results* table, the pile details (Map, 3D view) and
the exports show both the latest **survey Z** (with its date) and the **LIDAR Z**, the one in use
in bold (the survey when there is one), and their difference.

**Precision**: calculated values (tops of pile, cut/fill, set-out positions and shifts) are
rounded to the **centimetre** (*Settings → Tracker & reveal → Round … to*, 0.01 m; 0 keeps them
exact) and shown with 2 decimals; measured ground elevations keep 3. The rounding is part of the
calculation, not only of the display: each pile's top is chosen among the centimetre values next
to the exact optimum so that the angle limit, the reveal window, the flood minimum and the
cut/fill cap all still hold on the rounded values, and set-out positions are rounded before the
tracker is checked at them. On the project master file this gives 2,788 compliant and 4 graded
trackers (1 to 5 cm of cut/fill on two or three piles), against 2,791 and 1 with exact values.

The **average reveal** (Overview) is the mean height of the top of pile above the ground elevation
in use, over every pile with a result, with the lowest and highest values and the mean over the
surveyed piles only.

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

- Columns can be in any order. Headers are recognised in English and French (`Point`/`Nom`,
  `Easting`/`X`/`Est`, `Northing`/`Y`/`Nord`, `Elevation`/`Z`/`Altitude`/`TN`, `Code`, `Date`…),
  and the values then confirm or correct them:
  - the **pile ID column** is the one whose values match the most piles of the master list,
    wherever it is (e.g. point numbers first and the pile ID in the code/description column),
    including IDs inside text such as `P01.02.01.05_TN`;
  - **Easting / Northing** are the columns whose values equal those piles' design coordinates
    (so PENZD, PNEZD and swapped X/Y headers all work);
  - the **elevation** is the remaining numeric column closest to the site's ground levels.
- Files without a header row work the same way. The import report says which column was used
  for what.
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

### Set-out (new x, y of each pile)

The layout's distance between two consecutive piles is the length of the torque-tube span
between them (the layout is drawn flat: spans are nominal lengths such as 9.415 m). Once the tops
are optimised, a span between tops that differ by `dz` covers only `sqrt(L² - dz²)` on the ground,
so the piles must move along the tracker:

- the **motor pile** (pile type `HMP`/`SMP`…, or "Motor") keeps its design position; a tracker
  without exactly one motor pile is anchored on its middle pile;
- the other piles slide along the tracker axis towards it, span by span;
- the tracker is **re-optimised on the new spacing** until the positions move by less than
  0.1 mm, then **verified** at the final positions: true angle changes within the limit (else the
  tracker is *Non-compliant*), and every span's length along the tube equal to its design length
  (*Span check*).

Results show the set-out Easting/Northing and the shift (mm) of every pile, the fixed pile and the
largest shift of each tracker; piles moving more than 0.10 m are highlighted (Settings → Set-out,
where the calculation can also be turned off). On the project master file the largest shift of a
tracker is 11.5 mm (median), 98 mm (95th percentile) and 215 mm at most (steep 15-pile trackers).
The Excel/CSV exports carry `Set-out Easting`, `Set-out Northing`, `Shift Along Tracker (m)`, and
the ArcGIS export draws piles at their set-out position.

### ArcGIS

The **ArcGIS** button downloads a GeoPackage (`.gpkg`, OGC standard) with two layers:

- `piles`: 3D points at each pile's set-out position and top of pile (design position and ground
  when not optimised yet), with every attribute: IDs, tracker type, pile type and colour, design and
  set-out coordinates and shift, ground elevation and its source, survey offset, flood value /
  level / minimum top, cut/fill, ToP, reveal, angle change, tracker status;
- `trackers`: 3D lines through the pile tops, with status, largest angle and cut/fill, flood counts;
- `blocks`: the outline of each block (polygon), with its number of piles, trackers and surveyed
  piles.

The coordinate system is **GDA2020 / MGA zone 56 (EPSG:7856)** by default; another one can be set in
**Settings → ArcGIS export** (EPSG code). Then:

- **ArcGIS Pro**: *Add Data* → the `.gpkg` file (or drag it onto the map / a local scene).
- **ArcGIS Online / Portal**: *Content → New item* → the `.gpkg` → *Create a hosted feature layer*.
  To publish a newer export later and keep the same layer and its maps, open the layer's item
  page → *Update data* → *Overwrite entire layer* with the new file.
- **QGIS** opens it directly too.

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

### Shared database (Google Drive)

One database for the whole team, on any computer, in the desktop program or the browser version,
with no server: each computer keeps its own copy and exchanges it with one file, `tracker-top.db`,
in a shared folder. After every change (and every 30-60 s) a computer reads the shared file if it
changed, then writes its own when it has something new. When two computers changed things at the
same time, their changes are **merged**: imports are never edited, only added or voided, so the
merge is a union (each computer's new imports and voids, the most recent settings, the installers'
export log), followed by a re-optimisation. The only thing that cannot be merged is two different
master pile lists: the page then asks which database to keep, and keeps the other as a copy file in
the folder.

Set it up in **Settings → Shared database** on each computer: give the computer a name (shown as
"saved by"), then choose one of:

- **Folder on this computer** (Edge, Chrome): with
  [Google Drive for desktop](https://www.google.com/drive/download/) installed, choose a folder in
  your Google Drive, e.g. `G:\My Drive\Tracker-top` or a folder of a shared drive. Works the same
  with OneDrive or a network drive. After a reload the browser asks for permission again (one click
  on the cloud icon at the top; choose *Allow on every visit* when the browser offers it).
- **Google Drive online**: nothing to install; sign in with Google and paste the link of the Drive
  folder. This needs a Google "OAuth client ID" for the site's address, created once (below). Google
  asks to sign in again after an hour and after a reload (one click).

The first computer that connects writes its database to the folder; a computer with an empty
database takes the shared one; a computer with its own data merges it. The cloud icon next to the
database name at the top shows the state (*In step*, *Syncing…*, *Changes not in the shared copy
yet*, *Reconnect*, *Conflict*). Google Drive keeps earlier versions of `tracker-top.db` (in Drive:
the file's *Manage versions*). Do not open or edit `tracker-top.db` in the folder directly.

Notes: the desktop program shares while its page is open (watch-folder imports made while it is
closed are shared when the page opens). Each exchange moves the whole file (about 13 MB for the
36,913-pile project), which takes a few seconds on a site connection.

**Creating the Google OAuth client ID** (once, for *Google Drive online*; 10 minutes):

1. In the [Google Cloud console](https://console.cloud.google.com/), create a project (e.g.
   *tracker-top*), with the Google account that will own it.
2. *APIs & Services → Library*: enable the **Google Drive API**.
3. *Google Auth Platform* (formerly *OAuth consent screen*) → *Get started*: app name *tracker-top*,
   your e-mail. Audience **Internal** if your company uses Google Workspace (only its accounts can
   sign in, no review needed); otherwise **External**, and add each user's Google address under
   *Audience → Test users* (up to 100).
4. *Clients → Create client*: type **Web application**; under *Authorized JavaScript origins* add
   the site's address, `https://charles-bn.github.io`, and for the desktop program
   `http://localhost:8770`. No redirect URI is needed.
5. Copy the **Client ID** (`….apps.googleusercontent.com`) and either paste it in *Settings →
   Shared database → OAuth client ID* on each computer, or put it in `crates/top-app/web/config.js`
   (`googleClientId`) so that the page has it for everyone. A client ID is not a secret.

The page asks Google for access to the user's Drive files ("See, edit, create and delete…"),
because it must open the database file written by a colleague. With an *External* app in testing,
Google first shows "Google hasn't verified this app": *Continue*.

### Installers' files

When **every pile of a block** has a field-survey elevation and **every tracker** of it has a
current result (by default also: every tracker compliant or graded), the page writes the block's
Excel file for the installers into the shared folder, in **For installers** (Settings →
*Installers' files*: on/off, folder name, columns, the compliance condition).

*Where it is*: `For installers` is created automatically **inside the shared database folder**
(the one chosen in *Settings → Shared database*), next to `tracker-top.db`, when the first block
is complete; nothing else needs linking. Without a shared folder, no file is written (the
Overview says so). Files:
`Block 30 - pile heights rev 1.xlsx`, with a *Piles* sheet (by default: block, tracker, pile ID, pile
type, colour, set-out Easting / Northing, ground Z, ToP, reveal, tracker status, comment) and an
*About* sheet (block, revision, date, number of piles and trackers, how positions and tops were
made). In Google Drive, share just that folder with the installers.

Each block's file is written once, by whichever computer gets there first, and recorded in the
database (the Overview's *Installers' file* column shows *✓ rev 1* and the date). If a pile of the
block is surveyed again and its set-out position or top changes (to the millimetre), the next
revision is written next to it (`rev 2`), and its *About* sheet says which revision it replaces.
Re-optimising with the same results does not create a new revision.

### Custom export

**Custom…** (top right) opens a choice of:

- **columns** of the *Piles* sheet (30, grouped: pile, position, ground, flood, result), in the
  order of the chosen profile;
- **blocks** (piles and trackers of those blocks only);
- **sheets** (*Piles*, *Trackers*, *Imports*, *Settings*) and the format (Excel, or CSV of the
  piles).

A choice of columns and sheets can be saved as a **profile** (*Save as profile…*); profiles are
stored in the database, so they are shared with everyone using it. Two come with every database:
*Full* (everything) and *Installers* (the installers' files' columns).

### Map

The **Map** tab draws every pile in plan (design positions, north up), coloured by:

- **Survey coverage** (default): surveyed / not surveyed yet (LIDAR or design ground) / no elevation;
- **Survey date**: today, last 7 days, last 30 days, earlier, not surveyed;
- **Tracker status**, **Reveal** (around the nominal reveal), **Survey − LIDAR** (blue: survey lower
  than LIDAR, red: higher);
- **Installers' file** by block: sent, complete but not sent yet, not complete.

Each **block is outlined**: one line around all its piles, made only of straight segments along
the trackers (north-south) and across them (east-west), 2 m beyond the end piles and half a row
spacing beside the outer rows. Tracker ends within 3 m of each other are squared off into one
straight line, and notches one or two trackers wide are filled, always outwards, so the line
steps only where the trackers really do; a block crossed by an access road gets one line around
both parts (*Block outlines* turns them off). The legend gives counts and percentages; click a line to hide
or show it. Drag to move, scroll or
pinch to zoom, choose a block to zoom on it (the others fade), or type a tracker or pile ID. Click
a pile for its details (both ground values, top, reveal, set-out), with links to the Results and
the 3D view. *Save as image* downloads the map with its legend as a PNG.

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
crates/top-db     ...including api.rs, the request handler shared by the desktop and browser versions,
                  sync.rs (shared database: revisions, merge) and blocks.rs (installers' files)
crates/top-app    `tracker-top`: local web server (axum) + web page (web/, built into the exe);
                  web/sync.js moves the shared database to and from the folder or Google Drive
crates/top-web    browser version: the engine compiled to WebAssembly + worker/bridge (www/)
                  3D with three.js r170 and delaunator, bundled in web/vendor (MIT / ISC)
testdata/         small synthetic sample
docs/             how the code works
```

## Verification done

- Shared database, in a real browser (Chromium) with the project master file: the browser version and
  the desktop program sharing one folder (a stand-in for a Google Drive for desktop folder): the
  desktop's empty database takes the shared one; an import on one appears on the other; imports made
  on both at the same time are merged (same 902 surveyed piles and history on both); a different
  master list on a third computer is reported as a conflict and settled by keeping a copy; export
  profiles reach the other computer. The same with Google Drive online against a mock of the Drive
  API (sign-in, folder link, resumable upload of the 13 MB file, folder creation). The real Google
  sign-in and Drive API need the OAuth client ID and have not been tried yet.
- Installers' files on the project master file: block 30 (702 piles, 58 trackers) surveyed in two
  days → `Block 30 - pile heights rev 1.xlsx` written once (Piles sheet: 702 rows with the
  installers' columns; About sheet); one pile re-surveyed 0.35 m higher → `rev 2`, whose About sheet
  names the revision it replaces.
- Centimetre rounding on the project master file (36,913 piles): every top, cut/fill and set-out
  coordinate on a whole centimetre; largest angle change 0.70°, reveals 1.20–1.60 m, no tracker
  non-compliant; largest span-length difference from the rounded set-out 9.95 mm. 300 random
  trackers in the tests: every limit holds after rounding.
- Block outlines on the project master file: 41 blocks (the pile IDs' `PXX` agree with the Block
  column for all 36,913 piles), one line each, only north-south and east-west segments, every pile
  inside its own block's line (median 16 corners; 104 for block 29, which runs diagonally across
  the site); the `blocks` layer reads back with GDAL as 41 polygons in EPSG:7856.
- 2D map with the project master file in the desktop program and the browser version: coverage,
  survey date, status, reveal, survey − LIDAR and installers' modes, block zoom, pile details,
  image export; no errors.
- 39 database tests now (11 more: revision tracking, taking / merging / conflicts between copies, a
  desktop file and a browser copy with different page sizes, installers' files and revisions, export
  profiles, custom columns and blocks, average reveal).

- Browser version, in a real browser (Chromium) with the project master file: import and optimisation
  in 2.8 s, survey import, Excel / ArcGIS / CSV / database downloads, data and settings kept after a
  reload, empty database then restore from a downloaded copy; also served from a sub-folder as
  GitHub Pages does.
- 71 automated tests: 30 optimiser (solver on analytic problems, flat/sloped/dipped terrain, randomised
  trackers, parallel == serial, flood minimum raising the window / forcing fill / unreachable,
  set-out: motor pile detection, span lengths, angles at the set-out positions), 2 CLI, 39 database (file formats and header spellings, columns recognised from their values in any
  order, immutability triggers, duplicate files, ID and position matching, offset and jump checks,
  voiding, incremental re-runs, settings changes, flood layer in both modes, set-out, upgrade of a
  version 2 database in place, Excel and GeoPackage exports, the shared request handler, database
  copy and restore, shared database, installers' files, custom export).
- Set-out on the project master file: all 2,792 trackers anchored on their motor pile; an
  independent recheck from the exported Excel file finds every 3D span equal to its design length,
  no motor pile moved, and true angles within 0.70° at the set-out positions.
- GeoPackage export of the project master file read back with GDAL 3.12: `piles` (36,913 Point Z) and
  `trackers` (2,792 LineString Z) in EPSG:7856; the coordinate systems written (GDA2020 and GDA94 / MGA,
  WGS 84 and ETRS89 / UTM, Lambert-93) each read back as their EPSG definition.
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
- Browser version tested in Chromium (desktop Edge/Chrome); not yet on Firefox or Safari.
- Comparing as-built pile positions (survey after piling) with the set-out positions.
- Direct update of an ArcGIS Online layer (needs an ArcGIS credential).
- Databases made by the 0.2.0 preview (schema 1) are refused: start a new one and import the master
  list again. Databases of versions 0.3 to 0.5 (schemas 2 and 3) are upgraded in place; an older
  program refuses a database once a 0.6 program has opened it (schema 4), so update every computer
  that shares a database.
- Real Google sign-in / Drive API (needs the OAuth client ID, see *Shared database*).
