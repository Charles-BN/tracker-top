use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::time::Instant;

use top_core::csv::{read_piles, write_results};
use top_core::{
    default_solver, group_trackers, normalize_block, optimize_all, Params, TrackerStatus,
};

const HELP: &str = "\
top - pile top-of-pile (ToP) height optimiser for single-axis trackers

USAGE:
    top optimize --input <piles.csv> --output <results.csv> [OPTIONS]

INPUT CSV columns:
    required: Pile Name, Tracker, Easting, Northing, Z LIDAR
    optional: Block, Section, Pile Type

OPTIONS:
    -i, --input <file>        Input CSV
    -o, --output <file>       Output CSV
        --block <label>       Only process this block (\"01\" and \"1\" are the same)
        --max-angle <deg>     Max angle change between spans   [default: 0.70]
        --reveal <m>          Nominal reveal                   [default: 1.45]
        --delta-min <m>       Lowest reveal = nominal + this   [default: -0.25]
        --delta-max <m>       Highest reveal = nominal + this  [default: 0.15]
        --max-cut-fill <m>    Cap on cut/fill at any pile      [default: 1.0]
        --threads <n>         Worker threads                   [default: all cores]
    -h, --help                Show this help

EXIT CODE: 0 ok, 1 error, 3 some trackers could not be made compliant.
";

struct Args {
    input: String,
    output: String,
    block: Option<String>,
    threads: usize,
    params: Params,
}

fn parse_args(raw: Vec<String>) -> Result<Option<Args>, String> {
    let mut it = raw.into_iter();
    match it.next().as_deref() {
        Some("optimize") => {}
        Some("-h") | Some("--help") | None => return Ok(None),
        Some(other) => return Err(format!("unknown command \"{other}\" (try --help)")),
    }

    let mut input = None;
    let mut output = None;
    let mut block = None;
    let mut threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let mut params = Params::default();

    while let Some(flag) = it.next() {
        if flag == "-h" || flag == "--help" {
            return Ok(None);
        }
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        let num = |name: &str, v: String| {
            v.parse::<f64>().map_err(|_| format!("{name}: \"{v}\" is not a number"))
        };
        match flag.as_str() {
            "-i" | "--input" => input = Some(value("--input")?),
            "-o" | "--output" => output = Some(value("--output")?),
            "--block" => block = Some(normalize_block(&value("--block")?)),
            "--max-angle" => params.max_angle_deg = num("--max-angle", value("--max-angle")?)?,
            "--reveal" => params.reveal_nominal = num("--reveal", value("--reveal")?)?,
            "--delta-min" => params.delta_min = num("--delta-min", value("--delta-min")?)?,
            "--delta-max" => params.delta_max = num("--delta-max", value("--delta-max")?)?,
            "--max-cut-fill" => params.max_cut_fill = num("--max-cut-fill", value("--max-cut-fill")?)?,
            "--threads" => {
                let v = value("--threads")?;
                threads = v.parse().map_err(|_| format!("--threads: \"{v}\" is not an integer"))?;
            }
            other => return Err(format!("unknown option \"{other}\" (try --help)")),
        }
    }
    params.validate()?;
    Ok(Some(Args {
        input: input.ok_or("missing --input")?,
        output: output.ok_or("missing --output")?,
        block,
        threads,
        params,
    }))
}

fn run(args: Args) -> Result<bool, Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    let mut piles = read_piles(BufReader::new(File::open(&args.input)?))?;
    let total = piles.len();
    if let Some(b) = &args.block {
        piles.retain(|p| &p.block == b);
    }
    if piles.is_empty() {
        return Err(format!("no piles to process ({total} read, block filter: {:?})", args.block).into());
    }
    let n_piles = piles.len();
    let trackers = group_trackers(piles);
    let t_read = t0.elapsed();

    let t1 = Instant::now();
    let solver = default_solver();
    let results = optimize_all(&trackers, &args.params, &solver, args.threads);
    let t_opt = t1.elapsed();

    let mut out = BufWriter::new(File::create(&args.output)?);
    write_results(&mut out, &trackers, &results, &args.params)?;
    out.flush()?;

    let count = |s: TrackerStatus| results.iter().filter(|r| r.status == s).count();
    let max_angle = results.iter().fold(0.0_f64, |m, r| m.max(r.max_angle_deg));
    let max_cf = results.iter().fold(0.0_f64, |m, r| m.max(r.max_cut_fill));
    let flawed = results.iter().flat_map(|r| &r.piles).filter(|p| p.lidar_outlier).count();

    println!("Piles: {n_piles}   Trackers: {}   Threads: {}", trackers.len(), args.threads);
    println!(
        "Compliant: {}   Graded: {}   Non-compliant: {}   Skipped (<3 piles): {}   Invalid: {}",
        count(TrackerStatus::Compliant),
        count(TrackerStatus::Graded),
        count(TrackerStatus::NonCompliant),
        count(TrackerStatus::Skipped),
        count(TrackerStatus::InvalidInput),
    );
    println!(
        "Max angle change: {max_angle:.4} deg (limit {:.2})   Max cut/fill: {max_cf:.3} m   LIDAR outlier piles: {flawed}",
        args.params.max_angle_deg
    );
    println!(
        "Time: read {:.2}s, optimise {:.2}s   ->   {}",
        t_read.as_secs_f64(),
        t_opt.as_secs_f64(),
        args.output
    );

    let bad: Vec<&str> = trackers
        .iter()
        .zip(&results)
        .filter(|(_, r)| matches!(r.status, TrackerStatus::NonCompliant | TrackerStatus::InvalidInput))
        .map(|(t, _)| t.id.as_str())
        .collect();
    if !bad.is_empty() {
        let shown: Vec<_> = bad.iter().take(10).copied().collect();
        println!("Needs attention ({}): {}{}", bad.len(), shown.join(", "), if bad.len() > 10 { ", ..." } else { "" });
    }
    Ok(bad.is_empty())
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1).collect()) {
        Ok(None) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Some(args)) => match run(args) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(3),
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
