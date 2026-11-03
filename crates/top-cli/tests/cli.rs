use std::path::PathBuf;
use std::process::Command;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("top-cli-test-{}-{name}", std::process::id()))
}

#[test]
fn optimizes_a_small_csv_end_to_end() {
    let input = temp("in.csv");
    let output = temp("out.csv");

    // One tracker with a 70 cm dip (needs grading), one flat tracker, one 2-pile stub.
    let mut csv = String::from("Pile Name,Block,Tracker,Easting,Northing,Z LIDAR\n");
    for i in 0..15 {
        let z = if i == 7 { 99.3 } else { 100.0 };
        csv += &format!("A.{i:02},01,A,1000,{},{z}\n", 5000.0 + 8.0 * i as f64);
    }
    for i in 0..15 {
        csv += &format!("B.{i:02},01,B,1006,{},100\n", 5000.0 + 8.0 * i as f64);
    }
    for i in 0..2 {
        csv += &format!("C.{i:02},02,C,1012,{},100\n", 5000.0 + 8.0 * i as f64);
    }
    std::fs::write(&input, csv).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_top"))
        .args(["optimize", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .args(["--threads", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Trackers: 3"), "{stdout}");
    assert!(stdout.contains("Graded: 1"), "{stdout}");
    assert!(stdout.contains("Compliant: 1"), "{stdout}");
    assert!(stdout.contains("Skipped (<3 piles): 1"), "{stdout}");

    let results = std::fs::read_to_string(&output).unwrap();
    assert!(results.starts_with("Pile Name,Block,Section,Tracker,Pile Type,"));
    assert_eq!(results.lines().count(), 1 + 32);
    assert!(results.lines().any(|l| l.contains("A.07") && l.ends_with("Graded")));

    // Block filter: "2" must match the block written as "02" and keep only tracker C.
    let out = Command::new(env!("CARGO_BIN_EXE_top"))
        .args(["optimize", "--block", "2", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("Piles: 2   Trackers: 1"));

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
}

#[test]
fn reports_a_missing_column_with_exit_code_1() {
    let input = temp("bad.csv");
    std::fs::write(&input, "Pile Name,Tracker,Easting,Northing\nP1,T,1,2\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_top"))
        .args(["optimize", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(temp("never.csv"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Z LIDAR"));
    let _ = std::fs::remove_file(&input);
}
