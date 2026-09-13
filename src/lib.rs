pub mod build_info;
pub mod build_status;
mod close;
pub mod editor;
pub mod git;
pub mod mcp;
pub mod model;
pub mod native;
pub mod os;
pub mod panes;
pub mod search;
pub mod server;
pub mod tasks;
pub mod terminal;
pub mod ui;
pub mod wire;
pub mod workspace;

pub mod inbox_hook;
pub mod install;

pub mod attachments;
pub mod remote_files;
pub mod remote_protocol;
pub mod remote_services;
pub mod remote_update;

#[cfg(target_os = "linux")]
pub mod clipboard_owner;
pub mod image_preview;
pub mod screenshot;
pub mod screenshot_transfer;

pub mod diagnostics;

pub mod avatar;

pub mod pet;
// Shared bounded encoder for Flere-owned graphics.
#[path = "../companion/src/sixel.rs"]
pub mod sixel;
