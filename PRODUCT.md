# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Primary user: the site engineer on a utility-scale solar build, working from the site office on a Windows laptop. Each day they import the field survey, check which trackers have a valid top of pile, look into the ones flagged, and issue heights block by block to the pile installers. Other people (installers, surveyors, the design office, GIS) mostly receive its exports (Excel, CSV, GeoPackage) rather than use the interface.

## Product Purpose

tracker-top works out the top-of-pile (ToP) height of every pile on single-axis solar trackers. The heights keep each tracker within the project's design limits (reveal or tube-height window, angle change between spans, slope limits, flood freeboard, cut/fill cap), and it re-optimises automatically as daily surveys arrive. It also keeps the master pile database, the survey and import history, set-out positions, and the files that go to installers.

Success means every tracker gets compliant heights quickly and the engineer can trust them and trace them back to their source: which survey, which settings, which revision went to the installers.

## Positioning

A Rust port of the team's `SLSQP_ToP.py` optimiser, built into a self-contained tool rather than a script: one executable or one static web page, with no server and no install. It is aware of the tracker technology (Nextracker tops of pile, or ATI torque tube with a drop per pile type), aligns rows with their neighbours for backtracking, and keeps an append-only, auditable database that several computers share through a plain cloud folder.

## Operating Context

- Daily loop: survey files arrive (by hand, from a watch folder, or through Trimble Connect Sync). Trackers are re-optimised, the Overview shows what to look at, and Results shows profiles and pile tables. Blocks are ticked *Issued* once their heights go to the installers.
- Inputs are messy real-world files: CSV, TXT, Excel, headers in English or French, decimal commas, mixed separators.
- Outputs go into Excel, ArcGIS Pro / Online and QGIS, and into the installers' files.
- Several computers may share one database through Google Drive, OneDrive or a network folder. Merges, conflicts and the sync state are part of normal use.
- Scale: 37k to 76k piles and about 2.8k to 5k trackers per project.

## Capabilities and Constraints

- Internal tool, reused across the company's projects; it is not sold or marketed.
- Three ways to run it from one code base: the browser version (WASM, database in browser storage), the desktop `tracker-top` (serves the same page on localhost:8770 with a SQLite file), and the `top` command-line optimiser.
- **Must run on locked-down company PCs and offline**: no server, no install for the browser version, and vendored front-end libraries (no runtime CDN dependence). Future UI work must not add external runtime dependencies.
- **Large datasets must stay fast**: tables, the 2D map, the 3D view and plots must stay responsive at about 76k piles and 5k trackers.
- Data rules are part of the product: the master pile list never changes once loaded (new piles can only be added), imports are append-only and voidable, results are versioned and flagged out of date, and issued blocks keep their tops.
- Precision rules: calculated values are rounded to the centimetre (2 decimals) and measured ground keeps 3 decimals.
- Terminology to keep: pile, tracker, block, section, row, top of pile (ToP), reveal, torque tube / tube height (TTH), drop, motor / drive pile, cut/fill, Graded, Non-compliant, set-out, shift, flood level, freeboard, Issued, voided import, out of date.
- The UI is in English. Imported files may be in English or French.

## Brand Commitments

- Name: `tracker-top` (lowercase). There is an existing small logo mark (a sun, a tilted tube and piles) in `crates/top-app/web/index.html`.
- Voice, as in the README and the UI: plain, precise, engineering English. It names exact menu paths and real numbers and avoids marketing language.

## Evidence on Hand

- Validated project figures in the README (e.g. 2,788 compliant and 4 graded trackers on the master file; GESF ATI checks within 3 mm of the designer's Z Pile). These are real and may be quoted.
- `testdata/sample_block01_synthetic.csv`: synthetic sample data.
- There are no testimonials, users or external customers to cite, and none should be invented.

## Product Principles

1. **Trust through traceability.** Every number can be traced to its survey, settings and revision. Never hide which ground elevation or result is in use, or when it is out of date.
2. **Show what needs attention.** The daily job is to find the trackers that are not compliant, are steep, are out of date or are not surveyed yet, and to act on them. Surface those first.
3. **The engineer's words and units.** Use the site's terminology, metres with honest precision, and compass directions. No simplifying that loses engineering meaning.
4. **Works anywhere, stays fast.** Offline, no install, and responsive with tens of thousands of piles. Those limits are fixed and are not traded away for features.
5. **Data is never silently lost.** Imports, voids, merges and issued blocks are explicit and reversible where the rules allow, and are always confirmed when they are not.
