# Cubacadabra Desktop

The installed Cubacadabra player for macOS, Windows, and Linux. Desktop is
deliberately smaller than [Studio](../studio): it runs built game packages,
forwards native input to the shared client engine, renders the game, and
connects to the multiplayer Worker.

## Run locally

The player bundles the sibling `first-game` package during the Cargo build, so
the default launch needs no package argument:

    cargo run

To run another package during development, build it with the shared tools
repository and pass the optional path:

    cargo run --release --manifest-path ../tools/Cargo.toml --bin cubacadabra -- \
      build-game --source ../first-game --output /tmp/first-game-package

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
world and returns to the native player menu. The focused player menu can resume
the current package and opens the web control plane for Cube discovery, account,
avatar, safety, and other profile management. Set `CUBACADABRA_USERNAME` to
choose the initial local username before signing in.

## Repository boundaries

Desktop and Studio both depend on the shared Rust engine in `../rust`.
The Python game builder and native release workflow are shared from
`../tools`. Studio remains the authoring application; Desktop is the
runtime/player and does not depend on Studio.

## Licensing

Copyright (C) 2026 Andrew Arrow

Licensed under the GNU General Public License v3.0 or later.
See [LICENSE](LICENSE).
