<div align="center">
<h1>pwalarmd/pwalarmctl</h1>
</div>
<div align="center">

[![Crates version pwalarmd](https://img.shields.io/crates/v/pwalarmd)](https://crates.io/crates/pwalarmd) [![Crates version pwalarmctl](https://img.shields.io/crates/v/pwalarmctl)](https://crates.io/crates/pwalarmctl) [![GitHub version](https:/img.shields.io/github/v/release/amyipdev/pwalarmd)](https://github.com/amyipdev/pwalarmd/releases) [![License](https://img.shields.io/github/license/amyipdev/pwalarmd)](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html)

</div>

**pwalarmd** is a command-line (daemon-based) terminal alarm system. It has:
* ⏰ Configuration for as many alarms as you want
* 🎨 Lots of personalization options, including
  custom sounds and integration with your
  existing notification agent
* 🖥️ Support on both PipeWire and PulseAudio
* 💾 Easy configuration reload/save, including 
  through the `pwalarmctl` tool
  
## Installation

### Distro Packages

We don't have any distro packages yet. If you'd like
native distribution packages, help contribute!

### Manual Installation

#### Option 1: Crates.io

Install using `cargo`:

```sh
cargo install pwalarmd
cargo install pwalarmctl
```

#### Option 2: GitHub

Clone the repository:

``` sh
git clone https://github.com/amyipdev/pwalarmd.git
```

Enter and run the apps as needed:

``` sh
cd pwalarmd
# For pwalarmd
cargo run
# For pwalarmctl
cd pwalarmctl
cargo run -- <options>
```

Or, install to the system (not tested):

``` sh
cd pwalarmd
cargo install --path .
cd pwalarmctl
cargo install --path .
```

#### Post-Installation

You will need to configure your pwalarmd; this
includes providing sound assets.

## Configuration

You can set a custom config path by setting the
environment variable `PWALARMD_CONFIG` to the path.
All paths should be  **absolute** to avoid issues
with daemonization. Otherwise, `pwalarmd` first
looks for `~/.config/pwalarmd/pwalarmd.toml`,
then `/etc/pwalarmd.toml`.

If you're trying to troubleshoot or debug, set
`PWALARMD_NODAEMON=0` as an environment variable or
set `daemon = false` in your config.

## Usage

Run `pwalarmd` to launch the daemon.

Run `pwalarmctl` to control it, or modify the
config file currently being used. For help with
`pwalarmctl`, run `pwalarmctl help`.

To remove an alarm, run `pwalarmctl list`, and note
the 8 characters on the left of the alarm you want
to remove; you can then run `pwalarmctl remove N`,
where N are those characters, to remove the alarm.

## Contributing

Contributions are very much appreciated! There
isn't a formal contributor guide, but you can send
in a Pull Request; we do ask that you sign off
your commits by adding `Signed-off-by: Name <email>`
as the last line of your commit. 

If you find a bug or want new features, please
feel free to raise an Issue.

## Limitations and Known Issues

`pwalarmd` may crash under malformed packets. 
Work is being done to prevent this from happening.

## Licensing and Credits

This project was made by Amy Parker/amyipdev.

Copyright (C) 2024 Amy Parker, amy@amyip.net.

pwalarmd and pwalarmctl are licensed under the
GPLv. You can view the license in the LICENSE file.

This project is built in Rust, and uses several
Rust crates. You can see these crates in `Cargo.toml`
and `pwalarmctl/Cargo.toml`.

## Motivation

Windows, macOS, iOS, and Android all have usable
alarm subsystems. No such thing exists on most Linux
distros. There generally aren't any CLI-based ones,
with them all being GUI apps; they also often 
require that said GUI app be constantly open, 
which doesn't meet a lot of use cases.
