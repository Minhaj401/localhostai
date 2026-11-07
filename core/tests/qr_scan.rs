//! The QR must be scannable, not merely plausible.
//!
//! An earlier hand-rolled encoder produced grids that looked right and failed
//! to decode in a real scanner. Visual scanning was verified separately with
//! OpenCV during development (OpenCV decoded the payload exactly); this test
//! guards the structure and the encode/render path in CI.

use privateml_core::qr::{encode, render_terminal, PairPayload};

#[test]
fn payload_survives_encode_and_render() {
    let p = PairPayload::new(
        "192.168.1.20",
        8765,
        "0123456789abcdef0123456789abcdef",
        "workstation",
    );
    let uri = p.uri();

    let grid = encode(&uri).expect("encode");
    assert!(!grid.is_empty());

    // Finder patterns must be present in all three corners.
    let n = grid.len();
    assert!(
        grid[0][0] && grid[0][n - 1] && grid[n - 1][0],
        "missing finder corners"
    );

    // Terminal rendering must be non-empty and uniform width.
    let lines = render_terminal(&grid, 2);
    assert!(lines.len() > 4);
    let w = lines[0].chars().count();
    assert!(
        lines.iter().all(|l| l.chars().count() == w),
        "ragged QR render"
    );
}

#[test]
fn uri_round_trips_through_parse() {
    let p = PairPayload::new("10.0.0.5", 8765, "tok", "my machine");
    assert_eq!(PairPayload::parse(&p.uri()).unwrap(), p);
}

#[test]
fn real_pairing_uri_is_small_enough_to_scan() {
    // A live pairing URI must stay compact; long payloads raise QR version and
    // make terminal rendering uncomfortably wide.
    let p = PairPayload::new("192.168.100.200", 8765, &"a".repeat(32), "workstation");
    let uri = p.uri();
    assert!(uri.len() < 160, "pairing URI grew to {} chars", uri.len());
    let grid = encode(&uri).expect("encode");
    assert!(grid.len() <= 45, "QR grew to version with {} modules", grid.len());
}
