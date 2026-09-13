//! Real bridge input, image stream cancellation, and native-draft isolation.
use super::*;

fn image_finished(b: &mut Bridge, id: u64, label: &str, bytes: &[u8]) {
    let mut actual = Vec::new();
    b.until(|packet, _| {
        if packet.tag == protocol::PREVIEW_DATA {
            assert_eq!(packet.id, id);
            assert_eq!(
                u64::from_be_bytes(packet.data[..8].try_into().unwrap()),
                actual.len() as u64
            );
            actual.extend_from_slice(&packet.data[8..]);
        }
        packet.tag == protocol::PREVIEW_END && packet.id == id
    });
    assert_eq!(actual, bytes);
    b.wait_screen(label);
}

#[test]
fn gallery_vim_and_arrow_navigation_wraps_sorted_images_and_consumes_modal_input() {
    let f = Fixture::new();
    let (_, native_tab) = native(&f);
    let jpeg = include_bytes!("../fixtures/local-image.jpg");
    fs::write(f.root.join("Alpha.JPG"), jpeg).unwrap();
    fs::write(f.root.join("beta.png"), png()).unwrap();
    fs::write(f.root.join("preview.png"), png()).unwrap();
    fs::write(f.root.join("ignore.txt"), png()).unwrap();
    fs::create_dir(f.root.join("ignore.png")).unwrap();
    std::os::unix::fs::symlink(f.root.join("preview.png"), f.root.join("linked.png")).unwrap();
    let mut b = Bridge::new(&f, 160, 40);
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    select_preview(&mut b, &f);
    b.send(protocol::KEYS, 0, b"p");
    image_finished(&mut b, 1, "3/3 · preview.png", &png());
    let cases: &[(&[u8], &str, &[u8])] = &[
        (b"h", "2/3 · beta.png", &png()),
        (b"k", "1/3 · Alpha.JPG", jpeg),
        (b"\x1b[D", "3/3 · preview.png", &png()),
        (b"\x1b[A", "2/3 · beta.png", &png()),
        (b"l", "3/3 · preview.png", &png()),
        (b"j", "1/3 · Alpha.JPG", jpeg),
        (b"\x1b[C", "2/3 · beta.png", &png()),
        (b"\x1b[B", "3/3 · preview.png", &png()),
        (b"\x1bOC", "1/3 · Alpha.JPG", jpeg),
        (b"\x1bOD", "3/3 · preview.png", &png()),
        (b"\x1bOB", "1/3 · Alpha.JPG", jpeg),
        (b"\x1bOA", "3/3 · preview.png", &png()),
    ];
    for (index, (keys, label, bytes)) in cases.iter().enumerate() {
        let id = index as u64 + 2;
        b.send(protocol::KEYS, 0, *keys);
        b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == id - 1);
        image_finished(&mut b, id, label, bytes);
    }
    b.send(
        protocol::KEYS,
        0,
        b"\x1b[200~hjkl\r\x1b[201~\x1b[<0;5;5M\x1b[<0;5;5m",
    );
    b.send(protocol::KEYS, 0, b"\0");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 13);
    b.until(|packet, screen| {
        packet.tag == protocol::OUTPUT && !screen.capture(100).contains("IMAGE PREVIEW")
    });
    assert!(input(&f).is_empty());
    assert_eq!(f.snapshot().session().unwrap().run, native_tab.run);
    assert_eq!(f.snapshot().session().unwrap().pid, native_tab.pid);
}

#[test]
fn gallery_navigation_cancels_midstream_ignores_old_errors_and_recovers_from_deleted_file() {
    let f = Fixture::new();
    let (_, native_tab) = native(&f);
    let mut large = png();
    large.resize(10 * 1024 * 1024, 0);
    let first = f.root.join("preview.png");
    fs::write(&first, large).unwrap();
    fs::write(f.root.join("z-next.png"), png()).unwrap();
    let mut b = Bridge::new(&f, 160, 40);
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    select_preview(&mut b, &f);
    b.send(protocol::KEYS, 0, b"p");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_DATA && packet.id == 1);
    b.send(protocol::KEYS, 0, b"l");
    let mut old_ended = false;
    b.until(|packet, _| {
        old_ended |= packet.tag == protocol::PREVIEW_END && packet.id == 1;
        packet.tag == protocol::PREVIEW_CLEAR && packet.id == 1
    });
    assert!(!old_ended);
    image_finished(&mut b, 2, "2/2 · z-next.png", &png());
    b.send(protocol::PREVIEW_ERROR, 1, b"STALE_GALLERY_ERROR");
    b.send(protocol::RESIZE, 0, protocol::size_bytes(159, 40));
    b.screen = Terminal::new(159, 40);
    b.wait_screen("2/2 · z-next.png");
    assert!(!b.screen.capture(100).contains("STALE_GALLERY_ERROR"));
    fs::remove_file(&first).unwrap();
    b.send(protocol::KEYS, 0, b"h");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 2);
    b.wait_screen("No such file");
    assert!(b.screen.capture(100).contains("1/2 · preview.png"));
    b.send(protocol::KEYS, 0, b"l");
    image_finished(&mut b, 4, "2/2 · z-next.png", &png());
    b.send(protocol::PREVIEW_ERROR, 2, b"STALE_FINISHED_ERROR");
    b.send(protocol::KEYS, 0, b"q");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 4);
    assert!(input(&f).is_empty());
    assert_eq!(f.snapshot().session().unwrap().run, native_tab.run);
}

#[test]
fn gallery_clicked_absolute_path_navigates_siblings_and_closes_on_target_change() {
    let f = Fixture::new();
    let path = f.root.join("a clicked image.png");
    fs::write(&path, png()).unwrap();
    let jpeg = include_bytes!("../fixtures/local-image.jpg");
    fs::write(f.root.join("b next.JPEG"), jpeg).unwrap();
    let (workspace, native_tab) = native(&f);
    let mut b = Bridge::new(&f, 209, 49);
    // The polling native fixture must see the complete display, never the
    // briefly empty file between creation and its first write.
    flere::workspace::atomic_write(
        &f.root.join("image-display"),
        format!("\r\n({})\r\nDO_NOT_SUBMIT", path.display()).as_bytes(),
    )
    .unwrap();
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    b.wait_screen("DO_NOT_SUBMIT");
    let layout = flere::ui::Layout::new(209, 49);
    let (x, y) = (layout.terminal_x + 5, layout.terminal_y + 2);
    b.send(
        protocol::KEYS,
        0,
        format!("\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m"),
    );
    image_finished(&mut b, 1, "1/2 · a clicked image.png", &png());
    b.send(protocol::KEYS, 0, b"j");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 1);
    image_finished(&mut b, 2, "2/2 · b next.JPEG", jpeg);
    let other = f.new_workspace("Other gallery");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 2);
    b.wait_screen("Other gallery");
    let snapshot = f.snapshot();
    assert_eq!(snapshot.session().unwrap().run, other.run);
    let original = snapshot
        .workspaces
        .iter()
        .find(|w| w.id == workspace)
        .unwrap()
        .tabs
        .iter()
        .find(|s| s.id == native_tab.id)
        .unwrap();
    assert_eq!(
        (&original.run, original.pid),
        (&native_tab.run, native_tab.pid)
    );
    assert!(input(&f).is_empty());
}

#[test]
fn gallery_keys_during_initial_load_select_only_the_final_image() {
    let f = Fixture::new();
    native(&f);
    fs::write(f.root.join("preview.png"), png()).unwrap();
    let jpeg = include_bytes!("../fixtures/local-image.jpg");
    fs::write(f.root.join("z-next.jpg"), jpeg).unwrap();
    let mut b = Bridge::new(&f, 100, 30);
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    select_preview(&mut b, &f);
    // A single packet is processed before the worker can return. Navigation must
    // be remembered while the sibling list is unknown, never sent to the child.
    b.send(protocol::KEYS, 0, b"pllh");
    image_finished(&mut b, 2, "2/2 · z-next.jpg", jpeg);
    b.send(protocol::KEYS, 0, b"q");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 2);
    assert!(input(&f).is_empty());
}

#[test]
fn gallery_screenshot_keeps_full_view_and_the_finished_selected_image() {
    use flere::screenshot::{CELL_H, CELL_W};
    let f = Fixture::new();
    let (_, native_tab) = native(&f);
    fs::write(f.root.join("preview.png"), png()).unwrap();
    let selected = include_bytes!("../fixtures/local-image.png");
    fs::write(f.root.join("z-selected.png"), selected).unwrap();
    let mut b = Bridge::new(&f, 100, 30);
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    b.send(protocol::CAPABILITIES, 0, protocol::SCREENSHOT_CAP);
    b.send(
        protocol::CAPABILITIES,
        0,
        [
            protocol::AVATAR_SIZE_CAP,
            &protocol::size_bytes(CELL_W as u16, CELL_H as u16),
        ]
        .concat(),
    );
    select_preview(&mut b, &f);
    b.send(protocol::KEYS, 0, b"p");
    image_finished(&mut b, 1, "1/2 · preview.png", &png());
    b.send(protocol::KEYS, 0, b"l");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 1);
    image_finished(&mut b, 2, "2/2 · z-selected.png", selected);
    b.send(protocol::KEYS, 0, b"s");
    let mut receiver = flere::screenshot_transfer::Receiver::default();
    let (id, screenshot) = loop {
        let packet = b.until(|packet, _| {
            matches!(
                packet.tag,
                protocol::SCREENSHOT_BEGIN | protocol::SCREENSHOT_DATA | protocol::SCREENSHOT_END
            )
        });
        if let Some(screenshot) = receiver.packet(&packet).unwrap() {
            break (packet.id, screenshot);
        }
    };
    let (width, height, pixels) = flere::screenshot::decode(&screenshot).unwrap();
    assert_eq!((width, height), (100 * CELL_W, 30 * CELL_H));
    // The chosen 2x2 source has an opaque red top-left pixel. A text-only
    // screenshot or an overlay of the previous image cannot produce it here.
    let (x, y) = (49 * CELL_W, 3 * CELL_H);
    assert_eq!(
        &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4],
        &[255, 0, 0, 255]
    );
    b.send(protocol::SCREENSHOT_RESULT, id, [1]);
    b.send(protocol::KEYS, 0, b"h");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 2);
    image_finished(&mut b, 3, "1/2 · preview.png", &png());
    b.send(protocol::KEYS, 0, b"q");
    b.until(|packet, _| packet.tag == protocol::PREVIEW_CLEAR && packet.id == 3);
    assert!(input(&f).is_empty());
    assert_eq!(f.snapshot().session().unwrap().run, native_tab.run);
}
