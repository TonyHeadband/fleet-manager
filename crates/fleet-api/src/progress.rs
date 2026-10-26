//! Calendar arithmetic for release dates. Days are counted since 1970-01-01, UTC.

/// `"YYYY-MM-DD"` as days since 1970-01-01; None when it is not a real calendar date.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// The UTC day number of a Unix time in seconds.
pub fn today(now_secs: i64) -> i64 {
    now_secs.div_euclid(86_400)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's days-from-civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_date_rejects_feb_30() {
        assert_eq!(parse_date("2026-02-30"), None);
        assert_eq!(parse_date("2027-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-01"), None);
        assert_eq!(parse_date("2026-+1-01"), None);
    }

    #[test]
    fn parse_date_accepts_2028_02_29() {
        assert!(parse_date("2028-02-29").is_some());
        assert_eq!(
            parse_date("2028-03-01"),
            parse_date("2028-02-29").map(|d| d + 1)
        );
    }

    #[test]
    fn parse_date_round_trips_2026_10_26() {
        assert_eq!(parse_date("2026-10-26"), Some(20752));
        assert_eq!(today(20752 * 86_400 + 86_399), 20752);
        assert_eq!(parse_date("1970-01-01"), Some(0));
    }
}
