# Flere

Independent Rust experiment inspired by Switchyard; never modify or control the existing Switchyard installation, source or sessions.

- Work directly on `main`. The owner wants `main` to be the only local and remote branch; do not create feature branches.
- Keep UI, renderer, layout, navigation and explorer inside Flere. Small infrastructure crates (JSON, Unicode, OS bindings, storage and similar fundamentals) are allowed by explicit user instruction. Installed Git and the configured Vim/Neovim editor are allowed; no tmux, UI framework or Oil. External native shells/harnesses remain child programs.
- Linux x86_64 is the first supported target. The macOS arm64/x86_64 port has acceptance limits recorded in MACOS.md. Keep platform FFI behind target-specific code; do not claim complete cross-platform or VT compatibility.
- Keep unsafe FFI in `src/os.rs`, with explicit ABI assumptions and ownership comments.
- Build/test offline: `cargo test --offline`, `cargo clippy --offline --all-targets -- -D warnings`, `cargo build --offline --release`, `cargo fmt --check`.
- Use private state under the user's home; tests use disposable home-cache directories. No application state in source or system /tmp.
- Preserve shells across UI detach. Supervisor-only startup and background observation never launch a native chat. Opening a card restores only that card’s saved open tabs with their order/selection; other cards stay stopped until opened: exact recorded native conversation per tab, shell directory, or editor path. Unknown native IDs use the harness picker, never a latest-session guess or a Flere UUID form. Keep failed tabs for explicit retry; never replay drafts or commands. Retain stopped workspace metadata.
- Add project creates/reuses the main checkout card; additional project cards use linked Git worktrees. Primary is a directory distinction, never a privileged agent role.
- All agents are peers with the same coordination tools. Pinning is presentation only; never add a Lead role, forced pin or privileged coordinator. Retain exact request/run ownership and human acceptance checks.
- Socket operations require exact session/run identities. Untrusted terminal output must pass through the emulator, never be replayed as terminal escape sequences.
- No approval-bypass flags, native model test launches, publication or production migration without their separate authority.
