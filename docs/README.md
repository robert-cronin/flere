[← Flere](../README.md)

# The Flere handbook

Keep a working set of projects, shells, agents, and editor buffers in one terminal.
Start with the tour, learn a few navigation keys, then explore the workflows you need.

| Start here | Build your workflow | Understand the system |
| --- | --- | --- |
| [Quick start](getting-started.md) | [Workspaces, editors, and Git](guides/workspaces.md) | [Sessions and architecture](architecture.md) |
| [Watch the demos](demos.md) | [Native agents](guides/agents.md) | [Performance measurements](performance.md) |
| [Keyboard reference](reference/keyboard.md) | [Agent coordination](guides/coordination.md) | [Compatibility and limits](reference/compatibility.md) |
| [Troubleshooting](troubleshooting.md) | [Remote sessions](guides/remote.md) | [Build and contribute](development.md) |

**Looking for a command?** Open the [CLI reference](reference/cli.md), or run
`flere --help`. **Upgrading a running instance?** Read the
[session lifecycle](architecture.md#what-survives).

**Installing a release or using a package manager?** See
[downloads and distribution status](distribution.md).

## One distinction to learn first

A **workspace** is a project directory and its saved workflow metadata. A **tab**
is a live shell, agent, or editor process. The **UI** is an attachment to the
supervisor that owns those processes. Detaching the UI keeps your processes alive;
restarting the supervisor loads the saved tab layout without launching programs.
Opening the UI then reopens each agent tab with its own recorded conversation and
each shell in its saved directory, preserving order and selection.

## Current UI

See [workspace terminal trees and details](workspace-details.md) and the
[complete runtime guide](reference/runtime-guide.md) for Ghostline navigation,
local graphics, pets, Git trees and the latest file-loading behaviour.

## Engineering records

The [design record](../DESIGN.md), [validation record](releases/0.3.0-validation.md),
[macOS validation](../MACOS.md), [offline system evaluations](evaluations.md),
[coordination storage](design/coordination-storage.md),
and [remote development notes](../REMOTE-DEVELOPMENT.md)
retain detailed implementation and acceptance history. Historical records may
reference author-local evidence; portable demos and measurements live in this
handbook.
