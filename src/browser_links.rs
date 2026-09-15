//! Browser targets are inert display metadata. Only the companion's local input
//! handler can activate them; there is deliberately no remote open-URL command.
use serde::{Deserialize, Serialize};
use std::io;
pub const PROBE: &[u8] = b"[flere capability probe: card-links-v1]";
pub const CAPABILITY: &[u8] = b"card-links-v1";
pub const CONTEXT: u8 = 53;
pub const INPUT: u8 = 54;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub kind: &'static str,
    pub number: u64,
}
impl Link {
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > 2048 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        let path = value.strip_prefix("https://github.com/")?;
        let path = path.split(['?', '#']).next()?;
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 4
            || !parts[..2].iter().all(|s| {
                !s.is_empty()
                    && *s != "."
                    && *s != ".."
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return None;
        }
        let kind = match parts[2] {
            "issues" => "issue",
            "pull" => "pr",
            _ => return None,
        };
        if !parts[3].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number = parts[3].parse::<u64>().ok().filter(|n| *n > 0)?;
        Some(Self {
            url: format!(
                "https://github.com/{}/{}/{}/{number}",
                parts[0], parts[1], parts[2]
            ),
            kind,
            number,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: u16,
    pub y: u16,
    pub width: u16,
}
impl Region {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        y == self.y && x >= self.x && x < self.x.saturating_add(self.width)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub url: String,
    pub key: Option<u8>,
    pub region: Option<Region>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub input: u64,
    pub width: u16,
    pub height: u16,
    pub identity: String,
    pub targets: Vec<Target>,
}
impl Context {
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid browser link context");
        if bytes.len() > 32 * 1024 {
            return Err(invalid());
        }
        let context: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if !(10..=320).contains(&context.width)
            || !(8..=106).contains(&context.height)
            // Each wrapped link row is a target, including on a 10-column UI.
            || context.targets.len() > 32
            || context.identity.len() > 256
            || context.identity.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        for (index, target) in context.targets.iter().enumerate() {
            if !Link::parse(&target.url).is_some_and(|link| link.url == target.url)
                || target.key.is_none() && target.region.is_none()
                || target
                    .key
                    .is_some_and(|key| !matches!(key, b'i' | b'p' | b'\r'))
                || target.region.as_ref().is_some_and(|r| {
                    r.width == 0
                        || r.y >= context.height
                        || u32::from(r.x) + u32::from(r.width) > u32::from(context.width)
                })
            {
                return Err(invalid());
            }
            for other in &context.targets[..index] {
                if target.key.is_some() && target.key == other.key {
                    return Err(invalid());
                }
                if let (Some(a), Some(b)) = (&target.region, &other.region)
                    && a.y == b.y
                    && a.x < b.x + b.width
                    && b.x < a.x + a.width
                {
                    return Err(invalid());
                }
            }
        }
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_context_rejects_unsafe_ambiguous_and_out_of_bounds_targets() {
        let target = Target {
            url: "https://github.com/demo/fixture/issues/7".into(),
            key: Some(b'i'),
            region: Some(Region {
                x: 40,
                y: 5,
                width: 30,
            }),
        };
        let valid = Context {
            input: 0,
            width: 100,
            height: 30,
            identity: "fixture:1".into(),
            targets: vec![target],
        };
        let decode = |c: &Context| Context::decode(&serde_json::to_vec(c).unwrap());
        assert_eq!(decode(&valid).unwrap(), valid);
        for url in [
            "file:///tmp/test",
            "https://github.com@evil.test/demo/a/issues/7",
            "https://github.com/demo/a/issues/7?x=1",
            "https://github.com/demo/a/issues/$(command)",
            "https://github.com/demo/a/issues/7\n",
        ] {
            let mut c = valid.clone();
            c.targets[0].url = url.into();
            assert!(decode(&c).is_err());
        }
        let mut c = valid.clone();
        c.targets.push(c.targets[0].clone());
        assert!(decode(&c).is_err());
        let mut c = valid.clone();
        c.targets[0].region.as_mut().unwrap().width = 100;
        assert!(decode(&c).is_err());
        let mut c = valid.clone();
        c.targets[0].key = Some(b'q');
        assert!(decode(&c).is_err());
        let mut c = valid;
        c.targets[0].region = None;
        c.targets[0].key = None;
        assert!(decode(&c).is_err());
    }
}
