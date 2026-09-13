//! Optional companion-owned update flow. Requests carry no executable sources;
//! those are chosen with fresh local input and retained in a local plan.
pub const CAPABILITY: &[u8] = b"coordinated-update-v1";
pub const REQUEST: u8 = 41;
pub const ACTIVATE: u8 = 42;
pub const ACK: u8 = 43;
pub const RESULT: u8 = 44;
pub const CALL: u8 = 45;
pub const BEGIN: u8 = 46;
pub const DATA: u8 = 47;
pub const END: u8 = 48;
pub const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
pub fn token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
