# Cubacadabra Desktop

The installed Cubacadabra player for macOS, Windows, and Linux. Desktop is
deliberately smaller than [Studio](https://github.com/cubacadabra/studio): it runs built game packages,
forwards native input to the shared client engine, renders the game, and
connects to the multiplayer Worker.

This is a pre-launch player. Start with [Cuboom](https://github.com/cubacadabra/examples/blob/main/cuboom/README.md)
and the [platform contribution guide](https://github.com/cubacadabra/docs/blob/main/CONTRIBUTING.md).

## Run locally

Keep `desktop`, `rust`, `tools`, and `examples` as sibling checkouts. Install
stable Rust and a native C/C++ toolchain: Xcode Command Line Tools on macOS,
MSVC C++ build tools on Windows, or a compiler plus `pkg-config` and OpenSSL
development headers on Linux. A native graphics driver and desktop session
are required to play.

The player builds Cuboom, Schoolyard, and Signal Run from `../examples` during
the Cargo build. Cuboom is the default, so a local launch needs no package argument:

    cargo run

To run another package during development, build it with the shared tools
repository and pass the optional path:

    cargo run --release --manifest-path ../tools/Cargo.toml --bin cubacadabra -- \
      build-game --source ../examples/first-game --output /tmp/first-game-package

    cargo run -- --path /tmp/first-game-package

`--path <package-directory>` and a package directory as the first positional
argument remain available as development overrides. Debug builds default to the local Worker at
`http://127.0.0.1:8787` and local sign-in site at `http://127.0.0.1:5173`.
`cargo run --release` defaults to `https://api.cubacadabra.com` and
`https://cubacadabra.com/login/`. Set `CUBACADABRA_BACKEND_URL` and/or
`CUBACADABRA_WEB_URL` to override either environment explicitly.

Controls are `WASD` or the arrow keys to move, `Shift` to sprint, `Space` to
jump, left-drag to orbit the camera, and the mouse wheel to zoom.

Selecting **Leave Game** from the shared in-game menu disconnects the game
world and returns to the native player menu. The player menu can enter the
bundled Cuboom, Schoolyard, and Signal Run packages, or open **List cubes** to load
uploaded packages from the backend catalog. Account and informational surfaces
still open in the web control plane. Set `CUBACADABRA_USERNAME` to choose the
initial local username before signing in.

## Git hooks

Enable the repository's pre-commit hook once per checkout:

```sh
git config core.hooksPath .githooks
```

When a commit includes Rust source, the hook runs `cargo fmt --all` and
auto-stages formatting changes for Rust files that were already staged. Files
with separate unstaged changes must be staged or discarded before committing.

## Repository boundaries

Desktop and Studio both depend on the shared Rust engine in `../rust`.
The native game builder and native release workflow are shared from
`../tools`. Studio remains the authoring application; Desktop is the
runtime/player and does not depend on Studio.

## Licensing

Copyright (C) 2026 Andrew Arrow

Licensed under the GNU General Public License v3.0 or later.
See [LICENSE](LICENSE).
