//! Bounded local performance probe; no supervisor, shell or model is launched.
use flere::terminal::Terminal;
use std::time::Instant;
fn main() {
    let mut t = Terminal::new(120, 38);
    let start = Instant::now();
    for i in 0..100_000 {
        t.feed(
            format!(
                "\x1b[32mROW-{i:06} ordinary terminal history with a styled line 界\x1b[0m\r\n"
            )
            .as_bytes(),
        );
    }
    let feed_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let cloned = t.clone();
    let clone_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let image = serde_json::to_vec(&cloned).unwrap();
    let serialize_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let restored: Terminal = serde_json::from_slice(&image).unwrap();
    let deserialize_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(restored.history.len(), t.history.len());
    assert_eq!(
        restored.scrollback(Some(0), 0).encode(),
        t.scrollback(Some(0), 0).encode()
    );
    assert!(restored.capture(200).contains("ROW-099999"));
    let start = Instant::now();
    for i in 0..1000 {
        std::hint::black_box(t.scrollback(Some(i * 73), 0).encode());
    }
    let page_us = start.elapsed().as_secs_f64() * 1000.; // mean microseconds of 1000 pages
    let start = Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(t.capture(200));
    }
    let capture_us = start.elapsed().as_secs_f64() * 1000.;
    println!(
        "{}",
        serde_json::json!({"input_rows":100000,"retained_rows":t.history.len(),"compact_payload_bytes":t.history.bytes(),"serialized_terminal_bytes":image.len(),"feed_ms":feed_ms,"clone_ms":clone_ms,"serialize_ms":serialize_ms,"deserialize_ms":deserialize_ms,"page_mean_us":page_us,"capture_mean_us":capture_us})
    );
}
