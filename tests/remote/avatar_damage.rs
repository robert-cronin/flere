//! Real bridge frames consumed by the companion's production icon painter.
use super::*;
use flere::avatar::Layout;
#[path = "../../companion/src/avatars.rs"]
mod consumer;

fn decode(_: &[u8], area: (usize, usize)) -> std::io::Result<flere::sixel::Raster> {
    // Pixel decoding is unchanged; the packet, layout and Sixel writer are real.
    Ok(flere::sixel::Raster {
        width: area.0,
        height: area.1,
        rgb: vec![255; area.0 * area.1 * 3],
    })
}

#[test]
fn bridge_damage_frames_preserve_icons_across_text_updates_resize_and_menus() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&f.root)
            .status()
            .unwrap()
            .success()
    );
    fs::create_dir(f.root.join(".flere")).unwrap();
    fs::write(f.root.join(".flere/icon.png"), png()).unwrap();
    let mut b = Bridge::new(&f, 188, 39);
    b.wait_screen("IMAGE_NATIVE_DRAFT");
    let mut icons = consumer::Avatars::default();
    let mut painted = Vec::new();
    // An old companion's sized capability retains the old layout contract.
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 10, 0, 20]].concat(),
    );
    let mut image = false;
    let mut layout = false;
    b.until(|p, _| {
        match p.tag {
            protocol::OUTPUT => icons.begin_output(),
            protocol::AVATAR_IMAGE => {
                icons.receive(&p.data).unwrap();
                image = true;
            }
            protocol::AVATAR_LAYOUT_SIZED => {
                icons.sized_frame(&p.data, (188, 39)).unwrap();
                layout = true;
            }
            protocol::AVATAR_FRAME => panic!("unnegotiated avatar damage frame"),
            _ => {}
        }
        icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
        image && layout && !painted.is_empty()
    });
    b.send(protocol::CAPABILITIES, 0, protocol::AVATAR_DAMAGE_CAP);
    let frame = b.until(|p, _| p.tag == protocol::AVATAR_FRAME);
    let (initial, damaged) = Layout::decode_frame(&frame.data).unwrap();
    assert!(damaged);
    assert!(!initial.badges.is_empty());
    icons.damage_frame(&frame.data, (188, 39)).unwrap();
    painted.clear();
    icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
    assert!(!painted.is_empty());
    painted.clear();
    // Updating a notice changes text outside the icon slots.
    for i in 0..12 {
        let text = format!("text-only-frame-{i}");
        b.send(protocol::NOTICE, 0, text.as_bytes());
        let p = b.until(|p, screen| {
            if p.tag == protocol::OUTPUT {
                icons.begin_output();
            }
            p.tag == protocol::AVATAR_FRAME && screen.capture(100).contains(&text)
        });
        let (current, damaged) = Layout::decode_frame(&p.data).unwrap();
        assert_eq!(current, initial);
        assert!(!damaged);
        icons.damage_frame(&p.data, (188, 39)).unwrap();
        icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
        assert!(painted.is_empty(), "text updates must not retransmit icons");
    }
    // A same-grid resize clears pixels even though the layout stays identical.
    b.send(protocol::RESIZE, 0, protocol::size_bytes(188, 39));
    let p =
        b.until(|p, _| p.tag == protocol::AVATAR_FRAME && Layout::decode_frame(&p.data).unwrap().1);
    assert_eq!(Layout::decode_frame(&p.data).unwrap().0, initial);
    icons.damage_frame(&p.data, (188, 39)).unwrap();
    icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
    assert!(!painted.is_empty());
    painted.clear();
    b.send(protocol::KEYS, 0, b"\0 ");
    let p = b.until(|p, _| {
        p.tag == protocol::AVATAR_FRAME
            && Layout::decode_frame(&p.data).unwrap().0.badges.is_empty()
    });
    icons.damage_frame(&p.data, (188, 39)).unwrap();
    icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
    assert!(painted.is_empty());
    b.send(protocol::KEYS, 0, b"\x1b");
    let p = b.until(|p, _| {
        p.tag == protocol::AVATAR_FRAME
            && !Layout::decode_frame(&p.data).unwrap().0.badges.is_empty()
    });
    icons.damage_frame(&p.data, (188, 39)).unwrap();
    icons.paint(Some((10, 20)), &mut painted, decode).unwrap();
    assert!(!painted.is_empty());
    painted.clear();
    // Font changes must invalidate the pixels at the same grid size.
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 8, 0, 16]].concat(),
    );
    let p =
        b.until(|p, _| p.tag == protocol::AVATAR_FRAME && Layout::decode_frame(&p.data).unwrap().1);
    icons.damage_frame(&p.data, (188, 39)).unwrap();
    icons.paint(Some((8, 16)), &mut painted, decode).unwrap();
    assert!(!painted.is_empty());
    assert!(input(&f).is_empty());
    icons.suspend();
    b.disconnect();
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
}
