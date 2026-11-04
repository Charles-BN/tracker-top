//! Finding a survey date in a file name or a date cell.

use chrono::NaiveDate;

/// First plausible calendar date in `text`, as `YYYY-MM-DD`.
///
/// Understands `2026-11-03`, `2026_11_03`, `20261103`, `03-11-2026`,
/// `03.11.2026`, `03/11/2026` (day first, as used in Europe) and
/// `2026-11-03 14:05:00`.
pub fn find_date(text: &str) -> Option<String> {
    // Runs of digits with their positions.
    let mut groups: Vec<&str> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_ascii_digit(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                groups.push(&text[s..i]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        groups.push(&text[s..]);
    }

    let ymd = |y: &str, m: &str, d: &str| -> Option<NaiveDate> {
        let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
        if !(2000..=2100).contains(&y) {
            return None;
        }
        NaiveDate::from_ymd_opt(y, m, d)
    };

    for i in 0..groups.len() {
        let g = groups[i];
        if g.len() == 8
            && let Some(d) =
                ymd(&g[0..4], &g[4..6], &g[6..8]).or_else(|| ymd(&g[4..8], &g[2..4], &g[0..2]))
        {
            return Some(d.format("%Y-%m-%d").to_string());
        }
        if i + 2 < groups.len() {
            let (a, b, c) = (groups[i], groups[i + 1], groups[i + 2]);
            let found = match (a.len(), b.len(), c.len()) {
                (4, 1..=2, 1..=2) => ymd(a, b, c),
                (1..=2, 1..=2, 4) => ymd(c, b, a),
                _ => None,
            };
            if let Some(d) = found {
                return Some(d.format("%Y-%m-%d").to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_date;

    #[test]
    fn finds_dates_in_common_spellings() {
        assert_eq!(
            find_date("survey_2026-11-03.csv").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(
            find_date("TOPO 20261103 block 2.xlsx").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(
            find_date("leve 03.11.2026.csv").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(
            find_date("03112026_pieux.csv").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(
            find_date("2026-11-03 14:05:00").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(find_date("block 01 piles v2.csv"), None);
        assert_eq!(find_date("2026-13-40.csv"), None);
    }
}
