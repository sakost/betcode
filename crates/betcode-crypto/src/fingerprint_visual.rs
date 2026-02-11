//! Visual fingerprint representation for verification.
//!
//! Generates a deterministic ASCII art "randomart" image from a fingerprint,
//! similar to OpenSSH's key randomart. This makes it easier for users to
//! visually compare fingerprints across devices (e.g., via QR code scan
//! or side-by-side comparison).

use sha2::{Digest, Sha256};

/// Width of the randomart box (inner area).
const WIDTH: usize = 17;
/// Height of the randomart box (inner area).
const HEIGHT: usize = 9;

/// Characters representing cell visit density, ordered from sparse to dense.
///
/// Based on OpenSSH's randomart character set. Each character maps to a
/// visit count: space (0 visits), `.` (1), `o` (2), etc. The last two
/// characters (`S` and `E`) are reserved as position markers for the
/// bishop's start and end positions, respectively.
const CHARS: &[u8] = b" .o+=*BOX@%&#/^SE";

/// Index of the 'S' (start) marker in CHARS.
#[allow(clippy::cast_possible_truncation)] // CHARS.len() == 17, fits in u8
const START_MARKER_IDX: u8 = CHARS.len() as u8 - 2;
/// Index of the 'E' (end) marker in CHARS.
#[allow(clippy::cast_possible_truncation)] // CHARS.len() == 17, fits in u8
const END_MARKER_IDX: u8 = CHARS.len() as u8 - 1;
/// Maximum visit count index before the reserved marker positions.
const MAX_VISIT_IDX: u8 = START_MARKER_IDX - 1;

/// Generate an ASCII randomart image from a fingerprint string.
///
/// The algorithm is based on the "drunken bishop" method used by OpenSSH.
/// The bishop starts in the center of a grid and walks based on bits in the
/// fingerprint hash, incrementing cell visit counts as it goes.
///
/// Returns a multi-line string with a bordered box.
pub fn fingerprint_randomart(fingerprint: &str, title: &str) -> String {
    // Hash the fingerprint to get uniform input
    let hash = Sha256::digest(fingerprint.as_bytes());

    let mut field = [[0u8; WIDTH]; HEIGHT];
    let mut x: usize = WIDTH / 2;
    let mut y: usize = HEIGHT / 2;

    // Walk the bishop
    for byte in &hash {
        for shift in (0..8).step_by(2) {
            let bits = (byte >> shift) & 0x03;
            // Decode 2 bits into dx, dy
            let dx: i32 = if bits & 1 == 0 { -1 } else { 1 };
            let dy: i32 = if bits & 2 == 0 { -1 } else { 1 };

            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_possible_wrap,
                clippy::cast_sign_loss
            )]
            {
                x = (x as i32 + dx).clamp(0, (WIDTH - 1) as i32) as usize;
                y = (y as i32 + dy).clamp(0, (HEIGHT - 1) as i32) as usize;
            }

            if field[y][x] < MAX_VISIT_IDX {
                field[y][x] += 1;
            }
        }
    }

    // Mark start and end positions
    field[HEIGHT / 2][WIDTH / 2] = START_MARKER_IDX;
    field[y][x] = END_MARKER_IDX;

    // Build the output string
    let title_display = if title.len() > WIDTH {
        &title[..WIDTH]
    } else {
        title
    };
    let pad_left = (WIDTH - title_display.len()) / 2;
    let pad_right = WIDTH - title_display.len() - pad_left;

    let mut lines = Vec::with_capacity(HEIGHT + 2);
    lines.push(format!(
        "+{}{}{}+",
        "-".repeat(pad_left),
        title_display,
        "-".repeat(pad_right)
    ));

    for row in &field {
        let mut line = String::with_capacity(WIDTH + 2);
        line.push('|');
        for &cell in row {
            let idx = (cell as usize).min(CHARS.len() - 1);
            line.push(CHARS[idx] as char);
        }
        line.push('|');
        lines.push(line);
    }

    lines.push(format!("+{}+", "-".repeat(WIDTH)));
    lines.join("\n")
}

/// Format a fingerprint for human-readable display with grouping.
///
/// Takes a colon-separated hex fingerprint and groups it into lines
/// of 8 pairs each for easier reading.
pub fn format_fingerprint_display(fingerprint: &str) -> String {
    let parts: Vec<&str> = fingerprint.split(':').collect();
    let mut lines = Vec::new();
    for chunk in parts.chunks(8) {
        lines.push(chunk.join(":"));
    }
    lines.join("\n")
}

/// Generate a side-by-side comparison of two fingerprints.
///
/// Returns a formatted string showing both randomart images and whether
/// they match.
pub fn compare_fingerprints(
    local_fp: &str,
    local_label: &str,
    remote_fp: &str,
    remote_label: &str,
) -> (String, bool) {
    use std::fmt::Write;

    let matches = local_fp == remote_fp;
    let local_art = fingerprint_randomart(local_fp, local_label);
    let remote_art = fingerprint_randomart(remote_fp, remote_label);

    let local_lines: Vec<&str> = local_art.lines().collect();
    let remote_lines: Vec<&str> = remote_art.lines().collect();

    let mut output = String::new();
    let max_lines = local_lines.len().max(remote_lines.len());
    for i in 0..max_lines {
        let left = local_lines.get(i).copied().unwrap_or("");
        let right = remote_lines.get(i).copied().unwrap_or("");
        let _ = writeln!(output, "{left}  {right}");
    }

    if matches {
        output.push_str("\nFingerprints MATCH");
    } else {
        output.push_str("\nFingerprints DO NOT MATCH — possible MITM attack!");
    }

    (output, matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randomart_has_correct_dimensions() {
        let art = fingerprint_randomart("aa:bb:cc:dd:ee:ff", "test");
        let lines: Vec<&str> = art.lines().collect();
        // HEIGHT + 2 border lines
        assert_eq!(lines.len(), HEIGHT + 2);
        // Each line should be WIDTH + 2 border chars
        for line in &lines {
            assert_eq!(line.len(), WIDTH + 2);
        }
    }

    #[test]
    fn randomart_deterministic() {
        let fp = "aa:bb:cc:dd:ee:ff:11:22:33:44:55:66:77:88:99:00";
        let art1 = fingerprint_randomart(fp, "test");
        let art2 = fingerprint_randomart(fp, "test");
        assert_eq!(art1, art2);
    }

    #[test]
    fn randomart_different_for_different_fingerprints() {
        let art1 = fingerprint_randomart("aa:bb:cc", "test");
        let art2 = fingerprint_randomart("dd:ee:ff", "test");
        assert_ne!(art1, art2);
    }

    #[test]
    fn randomart_contains_start_and_end_markers() {
        let art = fingerprint_randomart("aa:bb:cc:dd:ee:ff", "test");
        assert!(art.contains('S'), "should contain start marker S");
        assert!(art.contains('E'), "should contain end marker E");
    }

    #[test]
    fn randomart_title_displayed() {
        let art = fingerprint_randomart("aa:bb:cc", "MyDaemon");
        assert!(art.contains("MyDaemon"));
    }

    #[test]
    fn randomart_long_title_truncated() {
        let long_title = "A".repeat(30);
        let art = fingerprint_randomart("aa:bb:cc", &long_title);
        let first_line = art.lines().next().unwrap();
        assert_eq!(first_line.len(), WIDTH + 2);
    }

    #[test]
    fn format_fingerprint_groups_into_lines() {
        let fp = (0..32)
            .map(|i| format!("{i:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        let display = format_fingerprint_display(&fp);
        assert_eq!(display.lines().count(), 4); // 32 parts / 8 per line = 4
    }

    #[test]
    fn compare_matching_fingerprints() {
        let fp = "aa:bb:cc:dd:ee:ff";
        let (output, matches) = compare_fingerprints(fp, "Local", fp, "Remote");
        assert!(matches);
        assert!(output.contains("MATCH"));
        assert!(!output.contains("DO NOT MATCH"));
    }

    #[test]
    fn compare_mismatched_fingerprints() {
        let (output, matches) = compare_fingerprints("aa:bb:cc", "Local", "dd:ee:ff", "Remote");
        assert!(!matches);
        assert!(output.contains("DO NOT MATCH"));
    }

    #[test]
    fn fingerprint_qr_string_is_deterministic() {
        // QR code would encode the fingerprint string; test that the
        // string representation is stable and scannable
        let fp = "aa:bb:cc:dd:ee:ff:11:22:33:44:55:66:77:88:99:00:ab:cd:ef:12:34:56:78:9a:bc:de:f0:12:34:56:78:9a";
        let display = format_fingerprint_display(fp);
        let display2 = format_fingerprint_display(fp);
        assert_eq!(display, display2);
        assert!(display.contains("aa:bb:cc:dd:ee:ff:11:22"));
    }

    #[test]
    fn fingerprint_comparison_detects_mismatch() {
        let fp1 = "aa:bb:cc:dd";
        let fp2 = "aa:bb:cc:ee"; // One byte different
        let (output, matches) = compare_fingerprints(fp1, "A", fp2, "B");
        assert!(!matches);
        assert!(output.contains("DO NOT MATCH"));
    }

    #[test]
    fn randomart_with_empty_fingerprint() {
        // Should not panic with empty input
        let art = fingerprint_randomart("", "empty");
        let lines: Vec<&str> = art.lines().collect();
        assert_eq!(lines.len(), HEIGHT + 2);
        for line in &lines {
            assert_eq!(line.len(), WIDTH + 2);
        }
    }

    #[test]
    fn randomart_with_empty_title() {
        let art = fingerprint_randomart("aa:bb:cc", "");
        let first_line = art.lines().next().unwrap();
        assert_eq!(first_line.len(), WIDTH + 2);
        // Should be all dashes between the + markers
        assert!(first_line.starts_with('+'));
        assert!(first_line.ends_with('+'));
    }

    #[test]
    fn randomart_single_bit_difference_produces_different_art() {
        let fp1 = "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99";
        let fp2 = "aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:98";

        let art1 = fingerprint_randomart(fp1, "test");
        let art2 = fingerprint_randomart(fp2, "test");

        assert_ne!(
            art1, art2,
            "single-byte change should produce different art"
        );

        // Count differing characters in the inner area (excluding borders)
        let lines1: Vec<&str> = art1.lines().skip(1).take(HEIGHT).collect();
        let lines2: Vec<&str> = art2.lines().skip(1).take(HEIGHT).collect();
        let mut diff_count = 0;
        for (l1, l2) in lines1.iter().zip(lines2.iter()) {
            for (c1, c2) in l1.chars().zip(l2.chars()) {
                if c1 != c2 {
                    diff_count += 1;
                }
            }
        }
        assert!(
            diff_count > 5,
            "small input change should produce multiple visual differences, got {diff_count}"
        );
    }

    #[test]
    fn format_fingerprint_single_pair() {
        let display = format_fingerprint_display("aa");
        assert_eq!(display, "aa");
    }

    #[test]
    fn format_fingerprint_empty_string() {
        let display = format_fingerprint_display("");
        assert_eq!(display, "");
    }
}
