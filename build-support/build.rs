//! An opaque stamp for this Cargo compile generation, not a source or payload digest.
//! Cargo can reuse this generation for direct `cargo rustc -- ...` arguments or
//! edits to external patched dependencies; package verification must bind the payload.
use serde_json::json;
use std::{
    collections::hash_map::RandomState,
    env, fs,
    hash::BuildHasher,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn watch(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn main() {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let manifest = Path::new(&manifest);
    let component = env::var("CARGO_PKG_NAME").expect("Cargo package name");
    let root = match component.as_str() {
        "flere" => manifest,
        "flere-connect" => manifest.parent().expect("companion repository root"),
        _ => panic!("unexpected Flere component"),
    };
    // These directory watches include embedded fonts, pet art and shell/editor hooks.
    // The companion imports core source outside its own manifest directory. The
    // registry core package intentionally has no sibling companion package.
    for relative in [
        "src",
        "companion/src",
        "build-support",
        "Cargo.toml",
        "Cargo.lock",
        "companion/Cargo.toml",
        "companion/Cargo.lock",
    ] {
        let path = root.join(relative);
        // Cargo treats a missing watched path as changed on every invocation.
        // Optional sibling source must not rotate a packaged core's cached stamp.
        if path.exists() {
            watch(&path);
        }
    }
    // Cargo itself tracks configuration and toolchain selection. Existing local files
    // are also explicit inputs; absent paths would force every no-op build to rerun.
    for relative in [
        ".cargo",
        "companion/.cargo",
        "rust-toolchain",
        "rust-toolchain.toml",
        "companion/rust-toolchain",
        "companion/rust-toolchain.toml",
    ] {
        let path = root.join(relative);
        if path.exists() {
            watch(&path);
        }
    }
    for key in [
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTC",
        "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_INCREMENTAL",
        "RUSTUP_TOOLCHAIN",
        "RUSTC_BOOTSTRAP",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let target = env::var("TARGET").expect("Cargo target");
    let profile = env::var("PROFILE").expect("Cargo profile");
    let target_key = target.to_ascii_uppercase().replace('-', "_");
    for suffix in ["RUSTFLAGS", "LINKER", "RUSTC_LINKER"] {
        println!("cargo:rerun-if-env-changed=CARGO_TARGET_{target_key}_{suffix}");
    }
    for profile_key in ["DEV", "RELEASE", "TEST", "BENCH"] {
        for suffix in [
            "OPT_LEVEL",
            "DEBUG",
            "SPLIT_DEBUGINFO",
            "STRIP",
            "DEBUG_ASSERTIONS",
            "OVERFLOW_CHECKS",
            "LTO",
            "PANIC",
            "INCREMENTAL",
            "CODEGEN_UNITS",
            "RPATH",
        ] {
            println!("cargo:rerun-if-env-changed=CARGO_PROFILE_{profile_key}_{suffix}");
        }
    }
    // Include explicitly selected custom profile/target settings too. Cargo handles
    // adding new configuration keys; these watches cover changes to existing inputs.
    for (key, _) in env::vars_os() {
        if let Some(key) = key.to_str()
            && (key.starts_with("CARGO_PROFILE_") || key.starts_with("CARGO_TARGET_"))
        {
            println!("cargo:rerun-if-env-changed={key}");
        }
    }
    let compiler = Command::new(env::var_os("RUSTC").expect("Cargo rustc"))
        .arg("-Vv")
        .output()
        .expect("read rustc identity");
    assert!(compiler.status.success(), "rustc identity command failed");
    let rustc = String::from_utf8(compiler.stdout).expect("rustc identity is UTF-8");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("build clock is after Unix epoch")
        .as_nanos();
    let pid = std::process::id();
    // RandomState is std's randomly seeded, non-cryptographic hasher. Together with
    // time and PID it distinguishes ordinary independent build generations. This is
    // deliberately neither a cryptographic digest nor proof of executable contents.
    let discriminator = RandomState::new().hash_one((timestamp, pid, &component, &target));
    let build_id = format!("{discriminator:016x}-{timestamp:x}-{pid:x}");
    let remote = "flere-remote-v6";
    let compatibility = if component == "flere" {
        json!({
            "control_identity": "flere-v4",
            "snapshot": {"current": 5, "read_min": 1, "read_max": 5},
            "refresh_handoff": {"current": 6, "read_min": 1, "read_max": 6},
            "saved_state": {"current": 7, "read_min": 2, "read_max": 7},
            "remote_protocol": {"current": remote, "accepts": [remote]}
        })
    } else {
        json!({"remote_protocol": {
            "current": remote,
            "accepts": ["flere-remote-v2", "flere-remote-v3", "flere-remote-v4", "flere-remote-v5", remote]
        }})
    };
    let info = json!({
        "schema_version": 1,
        "identity_kind": "cargo_generation_stamp",
        "component": component,
        "package_version": env::var("CARGO_PKG_VERSION").expect("Cargo package version"),
        "build_id": build_id,
        "target": target,
        "profile": profile,
        "rustc": rustc.trim(),
        "compatibility": compatibility
    });
    let out = env::var_os("OUT_DIR").expect("Cargo output directory");
    fs::write(
        Path::new(&out).join("flere-build-info.json"),
        serde_json::to_vec(&info).expect("serialize build identity"),
    )
    .expect("write embedded build identity");
    println!("cargo:rustc-env=FLERE_BUILD_ID={build_id}");
    println!("cargo:rustc-env=FLERE_BUILD_TARGET={target}");
}
