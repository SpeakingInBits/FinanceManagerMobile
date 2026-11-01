//! USD formatting and parsing over integer cents.

use crate::model::Cents;

/// Formats cents as a USD string with thousands separators, e.g. 123456 -> "$1,234.56".
pub fn format_cents(cents: Cents) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = (abs / 100).to_string();
    let mut grouped = String::with_capacity(dollars.len() + dollars.len() / 3);
    for (i, ch) in dollars.chars().enumerate() {
        if i > 0 && (dollars.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{sign}${grouped}.{:02}", abs % 100)
}

/// Parses a user-entered decimal string (e.g. "10.50", "$1,000") into cents. Like the web app's
/// `parseFloat`-based parser, it reads the longest numeric prefix and returns 0 when there is none.
pub fn parse_to_cents(input: &str) -> Cents {
    let normalized: String = input
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let mut end = 0;
    let mut seen_dot = false;
    let mut seen_digit = false;
    for (i, c) in normalized.char_indices() {
        match c {
            '-' if i == 0 => {}
            '.' if !seen_dot => seen_dot = true,
            d if d.is_ascii_digit() => seen_digit = true,
            _ => break,
        }
        end = i + 1;
    }
    if !seen_digit {
        return 0;
    }
    normalized[..end]
        .parse::<f64>()
        .map(|v| (v * 100.0).round() as Cents)
        .unwrap_or(0)
}

/// Shows cents in an amount field: empty for zero (so the placeholder shows), else "12.34".
pub fn cents_to_input(cents: Cents) -> String {
    if cents == 0 {
        String::new()
    } else {
        format!("{:.2}", cents as f64 / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_grouping_and_sign() {
        assert_eq!(format_cents(0), "$0.00");
        assert_eq!(format_cents(1050), "$10.50");
        assert_eq!(format_cents(123_456_789), "$1,234,567.89");
        assert_eq!(format_cents(-1999), "-$19.99");
        assert_eq!(format_cents(100_000), "$1,000.00");
    }

    #[test]
    fn parses_like_parse_float() {
        assert_eq!(parse_to_cents("10.50"), 1050);
        assert_eq!(parse_to_cents("$1,234.5"), 123_450);
        assert_eq!(parse_to_cents("  -3 "), -300);
        assert_eq!(parse_to_cents("1.2.3"), 120);
        assert_eq!(parse_to_cents("abc"), 0);
        assert_eq!(parse_to_cents(""), 0);
        assert_eq!(parse_to_cents("0.015"), 2);
    }

    #[test]
    fn input_text_hides_zero() {
        assert_eq!(cents_to_input(0), "");
        assert_eq!(cents_to_input(1234), "12.34");
    }
}
