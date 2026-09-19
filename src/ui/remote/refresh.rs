//! Preserve only an incomplete packet boundary across frontend exec, never keys.
use crate::{remote_protocol as protocol, wire};
use std::{
    io::{self, Read},
    process::Command,
};

const HANDOFF: &str = "FLERE_REMOTE_INPUT_TAIL";

// Full buffered frames can be discarded immediately. At most one partial frame
// remains: retain either its missing byte count or its incomplete length prefix.
// Payload bytes never enter the child environment or get replayed after refresh.
fn descriptor(mut buffered: &[u8]) -> io::Result<Option<String>> {
    while !buffered.is_empty() {
        if buffered.len() < 4 {
            return Ok(Some(format!("prefix:{}", wire::hex(buffered))));
        }
        let n = frame_length(buffered[..4].try_into().unwrap())?;
        if buffered.len() < n + 4 {
            return Ok(Some(format!("remaining:{}", n + 4 - buffered.len())));
        }
        buffered = &buffered[n + 4..];
    }
    Ok(None)
}

pub(in crate::ui) fn prepare(command: &mut Command, buffered: &[u8]) -> io::Result<()> {
    // A later refresh at a clean boundary must not inherit the previous tail.
    command.env_remove(HANDOFF);
    if let Some(value) = descriptor(buffered)? {
        // exec retains this PID. Descendants can inherit environment variables,
        // but must not consume this frontend's stream boundary on their own stdin.
        command.env(HANDOFF, format!("{};{value}", std::process::id()));
    }
    Ok(())
}

pub(super) fn discard_inherited(input: &mut impl Read) -> io::Result<usize> {
    match std::env::var_os(HANDOFF) {
        Some(value) => discard_handoff(
            input,
            value.to_str().ok_or_else(invalid)?,
            std::process::id(),
        ),
        None => Ok(0),
    }
}

fn discard_handoff(input: &mut impl Read, value: &str, pid: u32) -> io::Result<usize> {
    if value.len() > 26 {
        return Err(invalid());
    }
    let (owner, boundary) = value.split_once(';').ok_or_else(invalid)?;
    if owner.is_empty() || !owner.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let owner = owner.parse::<u32>().map_err(|_| invalid())?;
    if owner == 0 {
        return Err(invalid());
    }
    if owner != pid {
        return Ok(0);
    }
    discard(input, boundary)
}

fn invalid() -> io::Error {
    protocol::invalid("invalid remote refresh input boundary")
}
fn frame_length(header: [u8; 4]) -> io::Result<usize> {
    let n = u32::from_be_bytes(header) as usize;
    if !(9..=protocol::MAX).contains(&n) {
        return Err(invalid());
    }
    Ok(n)
}
fn discard(input: &mut impl Read, value: &str) -> io::Result<usize> {
    let (remaining, header_bytes) = if let Some(count) = value.strip_prefix("remaining:") {
        if count.is_empty() || count.len() > 5 || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let n = count.parse::<usize>().map_err(|_| invalid())?;
        if !(1..=protocol::MAX).contains(&n) {
            return Err(invalid());
        }
        (n, 0)
    } else if let Some(prefix) = value.strip_prefix("prefix:") {
        if !matches!(prefix.len(), 2 | 4 | 6) {
            return Err(invalid());
        }
        let prefix = wire::unhex(prefix).map_err(|_| invalid())?;
        let mut header = [0; 4];
        header[..prefix.len()].copy_from_slice(&prefix);
        input.read_exact(&mut header[prefix.len()..])?;
        (frame_length(header)?, 4 - prefix.len())
    } else {
        return Err(invalid());
    };
    // Read exactly the old frame tail. In particular, leave the new HELLO and
    // following capabilities in the unbuffered input for their normal readers.
    let mut buffer = [0; 4096];
    let mut left = remaining;
    while left != 0 {
        let n = left.min(buffer.len());
        input.read_exact(&mut buffer[..n])?;
        left -= n;
    }
    Ok(remaining + header_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Packet;

    fn packet(data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        Packet::new(protocol::KEYS, 0, data)
            .write(&mut bytes)
            .unwrap();
        bytes
    }

    #[test]
    fn every_partial_boundary_discards_only_the_old_packet_and_retains_no_payload() {
        let old = packet(b"OLD_DRAFT_MUST_NOT_REPLAY\r");
        let next = packet(b"FRESH_HANDSHAKE");
        for split in 1..old.len() {
            let mut buffered = packet(b"complete buffered frame");
            buffered.extend_from_slice(&old[..split]);
            let value = descriptor(&buffered).unwrap().unwrap();
            assert!(value.len() <= 15);
            assert!(!value.contains("DRAFT"));
            let tail = [&old[split..], &next].concat();
            let mut reader = tail.as_slice();
            assert_eq!(discard(&mut reader, &value).unwrap(), old.len() - split);
            assert_eq!(reader, next);
        }
        assert_eq!(descriptor(&[]).unwrap(), None);
        assert_eq!(descriptor(&old).unwrap(), None);
    }

    #[test]
    fn refresh_clears_inherited_boundary_when_nothing_partial_remains() {
        let mut command = Command::new("unused-test-command");
        command.env(HANDOFF, "remaining:1");
        prepare(&mut command, &packet(b"complete")).unwrap();
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == HANDOFF)
                .unwrap()
                .1,
            None
        );
    }

    #[test]
    fn inherited_handoff_only_consumes_the_reexec_frontends_input() {
        let mut input = b"old-new".as_slice();
        assert_eq!(
            discard_handoff(&mut input, "123;remaining:4", 456).unwrap(),
            0
        );
        assert_eq!(input, b"old-new");
        assert_eq!(
            discard_handoff(&mut input, "123;remaining:4", 123).unwrap(),
            4
        );
        assert_eq!(input, b"new");
        for value in [
            "123",
            "0;remaining:1",
            ";remaining:1",
            "+123;remaining:1",
            "4294967296;remaining:1",
        ] {
            assert!(discard_handoff(&mut input, value, 123).is_err());
            assert_eq!(input, b"new");
        }
    }

    #[test]
    fn malformed_or_truncated_boundaries_fail_without_scanning_for_a_new_frame() {
        for value in [
            "",
            "remaining:0",
            "remaining:65537",
            "remaining:+1",
            "remaining:1\n",
            "prefix:",
            "prefix:00000000",
            "prefix:xx",
            "unknown:1",
        ] {
            let mut input = b"unchanged".as_slice();
            assert!(discard(&mut input, value).is_err(), "{value}");
            assert_eq!(input, b"unchanged");
        }
        assert!(descriptor(&[0, 0, 0, 8]).is_err());
        assert!(descriptor(&[0, 1, 0, 1]).is_err());
        assert!(discard(&mut [0, 0, 8].as_slice(), "prefix:00").is_err());
        assert_eq!(
            discard(&mut [].as_slice(), "remaining:1")
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        let mut largest = vec![0; protocol::MAX];
        largest.push(42);
        let mut reader = largest.as_slice();
        assert_eq!(
            discard(&mut reader, "remaining:65536").unwrap(),
            protocol::MAX
        );
        assert_eq!(reader, [42]);
    }
}
