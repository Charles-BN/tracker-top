//! GeoPackage export (OGC GeoPackage 1.4), for ArcGIS Pro, ArcGIS Online, QGIS.
//!
//! Two layers:
//! - `piles`: one 3D point per pile at its set-out position and top of pile
//!   (design position and ground when it has no result yet), with every
//!   attribute of the Piles sheet;
//! - `trackers`: one 3D line per tracker through its pile tops, with status,
//!   largest angle, cut/fill and pile shift, flood counts.
//!
//! The coordinate system is the one set in Settings (EPSG code). Its definition
//! is written for the usual solar-site systems (WGS 84 / UTM, GDA94 and GDA2020
//! / MGA, ETRS89 / UTM, RGF93 / Lambert-93); for any other code only the code is
//! recorded, and ArcGIS may ask for the projection to be defined.

use crate::settings::Settings;
use crate::store::{Kind, PileState, State, TrackerState};
use rusqlite::{Connection, params};

const WGS84_WKT: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AXIS["Latitude",NORTH],AXIS["Longitude",EAST],AUTHORITY["EPSG","4326"]]"#;

struct Geog {
    name: &'static str,
    datum: &'static str,
    datum_code: u32,
    spheroid: &'static str,
    a: &'static str,
    rf: &'static str,
    spheroid_code: u32,
    code: u32,
    towgs84: bool,
}

const WGS84: Geog = Geog {
    name: "WGS 84",
    datum: "WGS_1984",
    datum_code: 6326,
    spheroid: "WGS 84",
    a: "6378137",
    rf: "298.257223563",
    spheroid_code: 7030,
    code: 4326,
    towgs84: false,
};
const GDA94: Geog = Geog {
    name: "GDA94",
    datum: "Geocentric_Datum_of_Australia_1994",
    datum_code: 6283,
    spheroid: "GRS 1980",
    a: "6378137",
    rf: "298.257222101",
    spheroid_code: 7019,
    code: 4283,
    towgs84: true,
};
const GDA2020: Geog = Geog {
    name: "GDA2020",
    datum: "Geocentric_Datum_of_Australia_2020",
    datum_code: 1168,
    spheroid: "GRS 1980",
    a: "6378137",
    rf: "298.257222101",
    spheroid_code: 7019,
    code: 7844,
    towgs84: false,
};
const ETRS89: Geog = Geog {
    name: "ETRS89",
    datum: "European_Terrestrial_Reference_System_1989",
    datum_code: 6258,
    spheroid: "GRS 1980",
    a: "6378137",
    rf: "298.257222101",
    spheroid_code: 7019,
    code: 4258,
    towgs84: true,
};
const RGF93: Geog = Geog {
    name: "RGF93 v1",
    datum: "Reseau_Geodesique_Francais_1993_v1",
    datum_code: 6171,
    spheroid: "GRS 1980",
    a: "6378137",
    rf: "298.257222101",
    spheroid_code: 7019,
    code: 4171,
    towgs84: true,
};

fn geogcs(g: &Geog) -> String {
    format!(
        r#"GEOGCS["{}",DATUM["{}",SPHEROID["{}",{},{},AUTHORITY["EPSG","{}"]],{}AUTHORITY["EPSG","{}"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","{}"]]"#,
        g.name,
        g.datum,
        g.spheroid,
        g.a,
        g.rf,
        g.spheroid_code,
        if g.towgs84 {
            "TOWGS84[0,0,0,0,0,0,0],"
        } else {
            ""
        },
        g.datum_code,
        g.code
    )
}

fn transverse_mercator(name: &str, g: &Geog, zone: u32, south: bool, code: u32) -> String {
    let cm = zone as i32 * 6 - 183;
    format!(
        r#"PROJCS["{name}",{},PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",{cm}],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",{}],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],AUTHORITY["EPSG","{code}"]]"#,
        geogcs(g),
        if south { 10_000_000 } else { 0 }
    )
}

/// Name and WKT of a projected coordinate system, for the codes this program knows.
pub fn crs_definition(epsg: u32) -> Option<(String, String)> {
    let (name, wkt) = match epsg {
        32601..=32660 => {
            let z = epsg - 32600;
            let n = format!("WGS 84 / UTM zone {z}N");
            let w = transverse_mercator(&n, &WGS84, z, false, epsg);
            (n, w)
        }
        32701..=32760 => {
            let z = epsg - 32700;
            let n = format!("WGS 84 / UTM zone {z}S");
            let w = transverse_mercator(&n, &WGS84, z, true, epsg);
            (n, w)
        }
        28348..=28358 => {
            let z = epsg - 28300;
            let n = format!("GDA94 / MGA zone {z}");
            let w = transverse_mercator(&n, &GDA94, z, true, epsg);
            (n, w)
        }
        7846..=7859 => {
            let z = epsg - 7800;
            let n = format!("GDA2020 / MGA zone {z}");
            let w = transverse_mercator(&n, &GDA2020, z, true, epsg);
            (n, w)
        }
        25828..=25838 => {
            let z = epsg - 25800;
            let n = format!("ETRS89 / UTM zone {z}N");
            let w = transverse_mercator(&n, &ETRS89, z, false, epsg);
            (n, w)
        }
        2154 => {
            let n = "RGF93 v1 / Lambert-93".to_string();
            let w = format!(
                r#"PROJCS["{n}",{},PROJECTION["Lambert_Conformal_Conic_2SP"],PARAMETER["latitude_of_origin",46.5],PARAMETER["central_meridian",3],PARAMETER["standard_parallel_1",49],PARAMETER["standard_parallel_2",44],PARAMETER["false_easting",700000],PARAMETER["false_northing",6600000],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],AUTHORITY["EPSG","2154"]]"#,
                geogcs(&RGF93)
            );
            (n, w)
        }
        _ => return None,
    };
    Some((name, wkt))
}

/// GeoPackage geometry blob: header (no envelope, little endian) + ISO WKB with Z.
fn gp_header(srs_id: i32) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, 0b0000_0001];
    b.extend_from_slice(&srs_id.to_le_bytes());
    b
}

fn point_z(srs_id: i32, x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut b = gp_header(srs_id);
    b.push(1);
    b.extend_from_slice(&1001u32.to_le_bytes());
    for v in [x, y, z] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// 2D multipolygon, one ring per polygon (block outlines).
fn multipolygon(srs_id: i32, rings: &[Vec<(f64, f64)>]) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, 0b0000_0001];
    b.extend_from_slice(&srs_id.to_le_bytes());
    b.push(1);
    b.extend_from_slice(&6u32.to_le_bytes());
    b.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for ring in rings {
        b.push(1);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(ring.len() as u32).to_le_bytes());
        for &(x, y) in ring {
            b.extend_from_slice(&x.to_le_bytes());
            b.extend_from_slice(&y.to_le_bytes());
        }
    }
    b
}

fn line_z(srs_id: i32, pts: &[(f64, f64, f64)]) -> Vec<u8> {
    let mut b = gp_header(srs_id);
    b.push(1);
    b.extend_from_slice(&1002u32.to_le_bytes());
    b.extend_from_slice(&(pts.len() as u32).to_le_bytes());
    for &(x, y, z) in pts {
        for v in [x, y, z] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}

/// Position drawn for a pile: its set-out position, else its design position.
fn draw_xy(p: &PileState) -> (f64, f64) {
    match p.result.as_ref().map(|r| (r.set_easting, r.set_northing)) {
        Some((Some(e), Some(n))) => (e, n),
        _ => (p.easting, p.northing),
    }
}

/// Elevation drawn for a pile: its top of pile, else its ground.
fn draw_z(p: &PileState) -> f64 {
    p.result.as_ref().map(|r| r.top).or(p.z).unwrap_or(0.0)
}

const PILE_COLUMNS: &str = "pile_id TEXT, tracker TEXT, block TEXT, section TEXT, tracker_type TEXT, pile_type TEXT, color TEXT,
    easting REAL, northing REAL, set_e REAL, set_n REAL, shift REAL, ground_z REAL, z_source TEXT, z_date TEXT, survey_offset REAL, n_surveys INTEGER,
    flood_value REAL, flood_level REAL, min_top REAL, raised_flood INTEGER,
    new_ground REAL, cut_fill REAL, top REAL, reveal REAL, angle_diff REAL,
    tracker_status TEXT, up_to_date INTEGER, run_id INTEGER, z_survey REAL, z_lidar REAL";

const TRACKER_COLUMNS: &str = "tracker TEXT, block TEXT, section TEXT, tracker_type TEXT, n_piles INTEGER, n_surveyed INTEGER,
    n_lidar INTEGER, n_missing INTEGER, status TEXT, up_to_date INTEGER, max_angle REAL, max_cut_fill REAL,
    fixed_pile TEXT, max_shift REAL, span_error_mm REAL,
    n_flood INTEGER, n_flood_raised INTEGER, run_id INTEGER, run_at TEXT, tube_slope_pct REAL, steep INTEGER";

fn up_to_date(t: &TrackerState) -> Option<i64> {
    t.run_id.map(|_| (!t.stale) as i64)
}

/// The current state as a GeoPackage file (bytes). Built in memory, so it
/// works without a file system (in the browser too).
pub fn geopackage(state: &State, settings: &Settings) -> Result<Vec<u8>, String> {
    let build = || -> rusqlite::Result<Vec<u8>> {
        let mut c = Connection::open_in_memory()?;
        write(&mut c, state, settings)?;
        let data = c.serialize("main")?;
        Ok(data.to_vec())
    };
    build().map_err(|e| format!("cannot write the GeoPackage: {e}"))
}

fn write(c: &mut Connection, state: &State, settings: &Settings) -> rusqlite::Result<()> {
    c.execute_batch(
        "PRAGMA application_id = 1196444487;  -- 'GPKG'
         PRAGMA user_version = 10400;
         CREATE TABLE gpkg_spatial_ref_sys (
           srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         CREATE TABLE gpkg_contents (
           table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE,
           description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
           min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER,
           CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id));
         CREATE TABLE gpkg_geometry_columns (
           table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL,
           CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name),
           CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name),
           CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id));",
    )?;
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84 geodetic', 4326, 'EPSG', 4326, ?1, 'longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid')",
        [WGS84_WKT],
    )?;
    tx.execute_batch(
        "INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', 'undefined cartesian coordinate reference system');
         INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', 'undefined geographic coordinate reference system');",
    )?;
    let srs_id: i32 = match settings.epsg {
        Some(code) if code != 4326 => {
            let (name, wkt) = crs_definition(code)
                .unwrap_or_else(|| (format!("EPSG:{code}"), "undefined".to_string()));
            tx.execute(
                "INSERT INTO gpkg_spatial_ref_sys VALUES (?1, ?2, 'EPSG', ?2, ?3, 'set in tracker-top Settings')",
                params![name, code, wkt],
            )?;
            code as i32
        }
        Some(_) => 4326,
        None => -1,
    };

    let stamp = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &state.piles {
        min_x = min_x.min(p.easting);
        min_y = min_y.min(p.northing);
        max_x = max_x.max(p.easting);
        max_y = max_y.max(p.northing);
    }
    if state.piles.is_empty() {
        (min_x, min_y, max_x, max_y) = (0.0, 0.0, 0.0, 0.0);
    }

    for (table, kind, columns, desc, z) in [
        (
            "piles",
            "POINT",
            PILE_COLUMNS,
            "Piles at their top of pile (Z), with ground, survey, flood and optimisation results",
            1,
        ),
        (
            "trackers",
            "LINESTRING",
            TRACKER_COLUMNS,
            "Trackers through their pile tops (Z), with status, largest angle and cut/fill",
            1,
        ),
        (
            "blocks",
            "MULTIPOLYGON",
            "block TEXT, piles INTEGER, trackers INTEGER, surveyed INTEGER",
            "Outline of each block, about 2 m outside its outer piles",
            0,
        ),
    ] {
        tx.execute_batch(&format!(
            "CREATE TABLE {table} (fid INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, geom {kind}, {columns});"
        ))?;
        tx.execute(
            "INSERT INTO gpkg_contents (table_name, data_type, identifier, description, last_change, min_x, min_y, max_x, max_y, srs_id)
             VALUES (?1, 'features', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![table, desc, stamp, min_x, min_y, max_x, max_y, srs_id],
        )?;
        tx.execute(
            "INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, ?3, ?4, 0)",
            params![table, kind, srs_id, z],
        )?;
    }

    {
        let mut ins = tx.prepare(
            "INSERT INTO piles (geom, pile_id, tracker, block, section, tracker_type, pile_type, color, easting, northing,
                set_e, set_n, shift,
                ground_z, z_source, z_date, survey_offset, n_surveys, flood_value, flood_level, min_top, raised_flood,
                new_ground, cut_fill, top, reveal, angle_diff, tracker_status, up_to_date, run_id, z_survey, z_lidar)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)",
        )?;
        for t in &state.trackers {
            for p in &state.piles[t.first_pile..t.first_pile + t.n_piles] {
                let r = p.result.as_ref();
                let (x, y) = draw_xy(p);
                ins.execute(params![
                    point_z(srs_id, x, y, draw_z(p)),
                    p.pile_id,
                    p.tracker,
                    p.block,
                    p.section,
                    p.tracker_type,
                    p.pile_type,
                    p.color,
                    p.easting,
                    p.northing,
                    r.and_then(|r| r.set_easting),
                    r.and_then(|r| r.set_northing),
                    r.and_then(|r| r.shift),
                    p.z,
                    p.z_source.map(|k| match k {
                        Kind::Survey => "Survey",
                        _ => "LIDAR / design",
                    }),
                    p.z_date,
                    p.offset_m,
                    p.n_surveys,
                    p.flood,
                    p.flood_level,
                    p.min_top,
                    r.map(|r| r.min_top_active as i64),
                    r.map(|r| r.new_ground),
                    r.map(|r| r.cut_fill),
                    r.map(|r| r.top),
                    r.map(|r| r.reveal),
                    r.map(|r| r.angle_diff_deg),
                    t.status,
                    up_to_date(t),
                    r.map(|r| r.run_id),
                    p.z_survey,
                    p.z_lidar,
                ])?;
            }
        }
        let mut ins = tx.prepare(
            "INSERT INTO trackers (geom, tracker, block, section, tracker_type, n_piles, n_surveyed, n_lidar, n_missing, status,
                up_to_date, max_angle, max_cut_fill, fixed_pile, max_shift, span_error_mm, n_flood, n_flood_raised, run_id, run_at,
                tube_slope_pct, steep)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
        )?;
        for t in &state.trackers {
            let pts: Vec<(f64, f64, f64)> = state.piles[t.first_pile..t.first_pile + t.n_piles]
                .iter()
                .map(|p| {
                    let (x, y) = draw_xy(p);
                    (x, y, draw_z(p))
                })
                .collect();
            let geom = (pts.len() >= 2).then(|| line_z(srs_id, &pts));
            ins.execute(params![
                geom,
                t.tracker,
                t.block,
                t.section,
                t.tracker_type,
                t.n_piles as i64,
                t.n_survey as i64,
                t.n_lidar as i64,
                t.n_missing as i64,
                t.status,
                up_to_date(t),
                t.max_angle_deg,
                t.max_cut_fill,
                t.anchor_pile,
                t.max_shift,
                t.span_error.map(|e| e * 1000.0),
                t.n_flood as i64,
                t.n_flood_raised as i64,
                t.run_id,
                t.run_at,
                t.tube_slope_pct,
                t.steep as i64,
            ])?;
        }
    }
    {
        let mut ins = tx.prepare(
            "INSERT INTO blocks (geom, block, piles, trackers, surveyed) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for o in crate::outline::block_outlines(state) {
            let ts = state.trackers.iter().filter(|t| t.block == o.block);
            let (trackers, surveyed) =
                ts.fold((0i64, 0i64), |(n, s), t| (n + 1, s + t.n_survey as i64));
            ins.execute(params![
                multipolygon(srs_id, &o.rings),
                o.block,
                o.piles as i64,
                trackers,
                surveyed
            ])?;
        }
    }
    tx.commit()?;
    c.execute_batch("VACUUM;")?;
    Ok(())
}
