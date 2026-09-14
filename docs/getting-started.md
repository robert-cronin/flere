[Documentation](README.md) · [Demos](demos.md) · [Keyboard](reference/keyboard.md)

# Your first five minutes

Flere 0.3.0 is a clean rename. Commands, package IDs, hooks, environment markers
and new private folders use `flere`; old Railhand installations are separate.

Choose the prebuilt terminal installer or Homebrew below. See
[installation channels](distribution.md) for other package-manager status.
The development workflow uses a source checkout.

## Install from your terminal

On **macOS Apple Silicon** or **Linux x86_64 GNU (glibc 2.39+)**, with
**Python 3.9 or newer** and either Wget or curl:

```sh
wget -qO- https://raw.githubusercontent.com/robert-cronin/flere/main/scripts/install.sh | sh
```

Or with curl:

```sh
curl -fsSL https://raw.githubusercontent.com/robert-cronin/flere/main/scripts/install.sh | sh
```

Both commands install the core and optional SSH/clipboard companion into
`~/.local/bin`. On a remote server that needs only the core, forward
`--core-only` to the installer:

```sh
wget -qO- https://raw.githubusercontent.com/robert-cronin/flere/main/scripts/install.sh | sh -s -- --core-only
"$HOME/.local/bin/flere" --version
```

The [shell bootstrap](../scripts/install.sh) downloads the published v0.3.0
Python installer over HTTPS and checks its pinned size and SHA-256 before running
it. That installer verifies packages from the current public release channel;
currently v0.3.0. It requires no Rust, source checkout, GitHub login or sudo,
and does not install system tools, edit PATH or start chats. Add `~/.local/bin`
to PATH to use `flere` directly. Git and Vim/Neovim are needed for their respective
workbench features.

Existing installations are not automatically adopted. Keep package-manager
installs under their manager; pass `--adopt` only when intentionally adopting an
existing manual user installation. Other installer arguments are forwarded
unchanged. Intel macOS prebuilt packages and Windows publication remain pending;
see [platform limits](reference/compatibility.md) and the source build below.

## Install with Homebrew

On macOS Apple Silicon or Linux x86_64, with a current Homebrew installation:

```sh
brew install robert-cronin/flere/flere
flere --version
```

For the optional local SSH and clipboard companion:

```sh
brew install robert-cronin/flere/flere-connect
flere-connect --version
```

The [published tap](https://github.com/robert-cronin/homebrew-flere) builds the
pinned v0.3.0 source locally and supplies Rust as a build dependency. Ensure
Homebrew's `bin` directory is on PATH, then [open a project](#open-a-project).
Use Homebrew for upgrades and removal. See the
[tap guide](../packaging/homebrew/README.md) for commands and
[validation limits](distribution.md#install-with-homebrew): the isolated macOS
lifecycle passed, while fresh dependency provisioning and native Linux Homebrew
lifecycle checks remain open.

## Build and install

From this repository checkout, use **Rust 1.98 or newer** and a system C linker.
On macOS, install Xcode Command Line Tools with `xcode-select --install` if needed.
Git and an installed Vim/Neovim are used for the corresponding workbench features.
Dependencies must already be cached for the offline checks below. On a new development machine, populate
the dependency cache once while online:

```sh
cargo fetch --locked
cargo fetch --locked --manifest-path companion/Cargo.toml
```

Then validate, package and install in one step:

```sh
./scripts/dev update --with-companion --state "$HOME/.local/state/flere"
"$HOME/.local/bin/flere" --version
```

The [developer script](../scripts/dev) validates offline, builds verified packages
and retains its evidence under the private home cache. To prepare artifacts
without installing, use `./scripts/dev package --with-companion` instead.
`update` installs and explicitly refreshes the selected running
supervisor while preserving sessions. It starts no supervisor when none is running.
`--with-companion` also validates/packages the Unix companion and installs it before
the core during update. Add `--adopt` only when intentionally adopting an existing
manual installation. `cargo build` by itself remains build-only.

Add `~/.local/bin` to PATH if you want to type just `flere`. Build natively on
Linux x86_64, Apple Silicon, or Intel macOS. The Apple Silicon port has local
runtime acceptance; Intel has compile-check evidence. See [platform limits](reference/compatibility.md).

### Check which build is running

Different development builds can share the same package version. Inspect the
command you installed and compare it with the supervisor for your selected state:

```sh
"$HOME/.local/bin/flere" --build-info
"$HOME/.local/bin/flere" --state "$HOME/.local/state/flere" build-status
```

Use your instance's actual `--state` path. These commands do not start or refresh
anything. `build-status` reports `same_build`, `different_build`, or `unknown`,
with `comparison_basis: "embedded_metadata"`. An older supervisor without build
metadata reports `unknown_build`; an unavailable socket reports `unreachable`.
The `executable` field describes the command you invoked, which may be a local
development binary rather than your installed command.

Build stamps are embedded Cargo generation identifiers, not source or executable
hashes. Standard source, shared asset, manifest, compiler and configuration changes
invalidate them. Some custom `cargo rustc` arguments or external patched dependency
changes can reuse Cargo's generated metadata. Package verification must separately
bind a candidate to its payload hash.

Managed installations report their verified package and activation receipt;
unmanaged installations and older/unregistered frontends remain explicitly unknown
or untracked. A matching supervisor does not establish that every UI or companion has updated.
Startup diagnostic logs include each component's own build stamp. The companion
also supports `flere-connect --build-info` without connecting to SSH.

### Update from Flere

This action owns Flere's per-user managed installation. If you installed through
Homebrew, Cargo or a system package manager, upgrade through that manager instead;
see [installation ownership](distribution.md#upgrade-through-the-installation-owner).

**Ctrl+Space, Shift+K** opens **Update Flere**. Locally, select a package directory
or HTTPS manifest and press Enter to apply; an empty source uses the explicitly
registered checkout. `scripts/dev update` registers its checkout. You can also
register one with `flere dev-source /absolute/path/to/flere`; the updater never
infers executable code from the active workspace directory.

Over SSH, the updated companion opens its own local source form for the remote core
and local companion. Enter prepares and verifies both; review their identities,
then Enter applies or Esc cancels. Exact sessions and drafts stay on the supervisor.
The result distinguishes installed files, the supervisor, the initiating frontend
and companion; other attachments are reported separately. See [remote setup](guides/remote.md).

Old Railhand supervisors cannot be refreshed with a Flere binary. Start a new
Flere instance instead. **Ctrl+Space, Shift+R** remains an explicit refresh action,
separate from installing a new package.

### Install a published package

The v0.3.0 Unix
[install.py](../scripts/install.py) fetches the platform’s core and companion
together from the Flere release channel. It checks
both payload hashes and matching source/version/protocol before installing either.
Use `--core-only` on a remote server, or supply a core manifest URL and
`--companion-url` for a custom HTTPS channel. Each component records its own
installation; the report identifies any partial failure so it can be repaired.

Windows uses [install-companion.ps1](../scripts/install-companion.ps1) with
`-ManifestUrl`, installing both `flere.exe` and the `flere-connect.exe` alias.
These installers work without Rust or a source checkout. They do not change PATH,
start chats or publish a release. The default public channel currently provides
Linux x86_64 GNU (glibc 2.39+) and macOS arm64; Windows publication is pending
physical acceptance. If the release or target is unavailable, use the
source-build instructions above or an explicit verified package source.

## Open a project

In an ordinary terminal, change to your project directory and run:

```sh
flere
```

First opening shows the intro. Press any key to begin. Flere creates a shell
workspace in that directory. Reopening populated
state connects to its supervisor. A new agent starts when you explicitly
request it.

Your first useful keys:

| Action | Keys |
| --- | --- |
| Enter navigation | **Ctrl+Space** |
| Move between panes | **h / l** in navigation |
| Cycle centre-pane tabs | **Tab / Shift+Tab** in navigation |
| Add a project at its Primary checkout | **G** in navigation |
| Add a worktree card | **n** in navigation |
| Add a shell tab | **t** in navigation |
| Close a tab | **x** in navigation, or its close button |
| Copy the whole Flere view | **s** in navigation |
| Open Git | **g** in navigation |
| Quick Open / Find in Files | **Q / H** in navigation |
| Run a task / inspect Problems | **! / ;** in navigation |
| Update Flere | **K** in navigation |
| Return to terminal input | **Enter**, with Terminal focused |
| Detach and keep processes alive | **q** in navigation |

Ctrl+Space enters a navigation mode; the following key is a separate press.
On a Mac, check the input-source shortcut if the OS consumes Ctrl+Space. See
[troubleshooting](troubleshooting.md#ctrlspace-does-nothing).

An empty Zsh prompt or a clean Vim/Neovim tab opened by Flere can close without
a dialog. Unsaved buffers, shell drafts, jobs, chats, and unverifiable state still
ask for confirmation. Existing tabs keep the confirmation behavior until reopened;
the integrations are loaded when a new shell or editor starts.

To share the whole Flere view as an image, press **Ctrl+Space**, then **s**, or
search Actions for **Copy Flere screenshot**. Wait for “Flere screenshot
copied,” then use your terminal's paste shortcut (**⌘V** in Ghostty). Flere
includes the side panes, tabs and status bar, and follows displayed scrollback.
The action returns to buffer input so you can paste immediately. The clipboard
offers the PNG to image-aware apps and its saved path to terminals. Local macOS
and Linux are supported; SSH uses the updated Windows/macOS/Linux companion.
See [remote setup](guides/remote.md). Rendering stays inside Flere and uses a
bundled font, which may differ from your terminal's configured font.

To split the centre, open **Ctrl+Space, Space** and search **Split**. Choose right
or below, with the current tab or a new shell. Each pane has its own tabs.
In navigation, **Tab / Shift+Tab** cycles the focused pane's tabs; directional
keys switch between panes. Drag the divider, use **Shift+Z** to zoom one pane,
or **Shift+X** to remove the split while keeping every terminal. Attached windows
share the split arrangement, focus and selections.

After five minutes idle, the original intro returns as a screensaver with a small
floating Flere by default. Explicit saved mascot choices remain unchanged.
A keyboard press or paste wakes it without sending that first input to a terminal.
Mouse movement and clicks keep it open; click to pet, or drag and throw the mascot.
Use **Ctrl+Space, Shift+U** to start it now, or **Ctrl+Space, Shift+V** to change the
idle setting. **Ctrl+Space, ^** selects Flere for both the intro and sidebar pet
and immediately previews the intro; **Ctrl+Space, %** selects Duck. These choices
preserve whether the sidebar pet is enabled. Your sessions keep running underneath.

## Pick up where you left off

Press **Ctrl+Space**, then **q**. Run `flere` again to reconnect to the same
shells and drafts. A system reboot or supervisor crash has different semantics:
saved workspace metadata remains, but stopped processes need an explicit start.

## Try a separate instance

Use an explicit state path to keep a trial separate from your normal workbench:

```sh
flere --state "$HOME/.local/state/flere-trial"
```

Use that same `--state` value for every command targeting the trial. All attachments
to one state share its active workspace and viewport.

Next: [watch the tour](demos.md), [open an editor or Git comparison](guides/workspaces.md),
or [start a native agent](guides/agents.md).
