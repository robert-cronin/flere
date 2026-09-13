//! Identity embedded at compile time; replacing a running executable's path cannot change it.
//! The opaque build stamp is diagnostic provenance, not a source or payload digest.
//! Cargo may reuse it for direct `cargo rustc -- ...` arguments or edits to external
//! patched dependencies. Matching metadata does not establish identical binary bytes.

pub const BUILD_ID: &str = env!("FLERE_BUILD_ID");
pub const TARGET: &str = env!("FLERE_BUILD_TARGET");

pub fn json() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/flere-build-info.json"))
}
