//! One bounded screenshot per foreground connection, with exact transfer IDs.
use crate::{image_preview, remote_protocol as p};
use std::{
    io,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Receiver {
    last: u64,
    pending: Option<Pending>,
}
struct Pending {
    id: u64,
    size: usize,
    bytes: Vec<u8>,
    started: Instant,
}
impl Receiver {
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    pub fn packet(&mut self, packet: &p::Packet) -> io::Result<Option<Vec<u8>>> {
        if self
            .pending
            .as_ref()
            .is_some_and(|v| v.started.elapsed() > Duration::from_secs(30))
        {
            self.cancel();
            return Err(p::invalid("screenshot transfer timed out"));
        }
        match packet.tag {
            p::SCREENSHOT_BEGIN => {
                if self.pending.is_some()
                    || packet.id <= self.last
                    || packet.id == 0
                    || packet.data.len() != 8
                {
                    return Err(p::invalid("invalid screenshot offer"));
                }
                let size = u64::from_be_bytes(packet.data[..].try_into().unwrap());
                if !(33..=p::IMAGE_LIMIT).contains(&size) {
                    return Err(p::invalid("screenshot exceeds size bound"));
                }
                self.last = packet.id;
                self.pending = Some(Pending {
                    id: packet.id,
                    size: size as usize,
                    bytes: Vec::new(),
                    started: Instant::now(),
                });
            }
            p::SCREENSHOT_CANCEL => {
                if self.pending.as_ref().is_some_and(|v| v.id == packet.id) {
                    self.cancel();
                }
            }
            p::SCREENSHOT_DATA | p::SCREENSHOT_END => {
                let v = self
                    .pending
                    .as_mut()
                    .filter(|v| v.id == packet.id)
                    .ok_or_else(|| p::invalid("screenshot has no matching offer"))?;
                if packet.tag == p::SCREENSHOT_DATA {
                    if packet.data.len() <= 8
                        || packet.data.len() > p::CHUNK + 8
                        || u64::from_be_bytes(packet.data[..8].try_into().unwrap())
                            != v.bytes.len() as u64
                        || v.bytes.len() + packet.data.len() - 8 > v.size
                    {
                        return Err(p::invalid("invalid screenshot chunk"));
                    }
                    v.bytes.extend_from_slice(&packet.data[8..]);
                } else {
                    if !packet.data.is_empty() || v.bytes.len() != v.size {
                        return Err(p::invalid("incomplete screenshot"));
                    }
                    let v = self.pending.take().unwrap();
                    if !v.bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                        return Err(p::invalid("screenshot must be PNG"));
                    }
                    image_preview::dimensions(&v.bytes)?;
                    return Ok(Some(v.bytes));
                }
            }
            _ => return Err(p::invalid("unknown screenshot packet")),
        }
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_receives_only_complete_current_ordered_images() {
        let bytes = include_bytes!("../tests/fixtures/local-image.png");
        let mut r = Receiver::default();
        assert!(r.packet(&p::Packet::new(p::SCREENSHOT_END, 1, [])).is_err());
        r.packet(&p::Packet::new(
            p::SCREENSHOT_BEGIN,
            1,
            (bytes.len() as u64).to_be_bytes(),
        ))
        .unwrap();
        assert!(r.packet(&p::Packet::new(p::SCREENSHOT_END, 1, [])).is_err());
        let mut chunk = 0u64.to_be_bytes().to_vec();
        chunk.extend(bytes);
        assert!(
            r.packet(&p::Packet::new(p::SCREENSHOT_DATA, 2, chunk.clone()))
                .is_err()
        );
        assert!(
            r.packet(&p::Packet::new(p::SCREENSHOT_DATA, 1, chunk.clone()))
                .unwrap()
                .is_none()
        );
        assert!(
            r.packet(&p::Packet::new(p::SCREENSHOT_DATA, 1, chunk))
                .is_err()
        );
        assert_eq!(
            r.packet(&p::Packet::new(p::SCREENSHOT_END, 1, []))
                .unwrap()
                .unwrap(),
            bytes
        );
        assert!(
            r.packet(&p::Packet::new(
                p::SCREENSHOT_BEGIN,
                1,
                (bytes.len() as u64).to_be_bytes()
            ))
            .is_err()
        );
        r.packet(&p::Packet::new(
            p::SCREENSHOT_BEGIN,
            2,
            (bytes.len() as u64).to_be_bytes(),
        ))
        .unwrap();
        r.packet(&p::Packet::new(p::SCREENSHOT_CANCEL, 2, []))
            .unwrap();
        assert!(r.packet(&p::Packet::new(p::SCREENSHOT_END, 2, [])).is_err());
    }
}
