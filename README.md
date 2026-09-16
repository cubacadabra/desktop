# Cubacadabra Desktop

The installed Cubacadabra player for macOS, Windows, and Linux. Desktop is
deliberately smaller than [Studio](../studio): it runs built game packages,
forwards native input to the shared client engine, renders the game, and
connects to the multiplayer Worker.

## Run locally

Build a package with the shared tools repository, then run it:

    PYTHONPATH=../tools/src python3 -m cubacadabra build-game \
      --source ../first-game \
      --output /tmp/first-game-package

    cargo run --release -- --path /tmp/first-game-package

The player accepts either `--path <package-directory>` or a package directory
as its first positional argument. The backend defaults to the local Worker;
set `CUBACADABRA_BACKEND_URL` for another environment.

Controls are `WASD` or the arrow keys to move, `Shift` to sprint, `Space` to
jump, left-drag to orbit the camera, and the mouse wheel to zoom.

## Repository boundaries

Desktop and Studio both depend on the shared Rust engine in `../rust`.
The Python game builder and native release workflow are shared from
`../tools`. Studio remains the authoring application; Desktop is the
runtime/player and does not depend on Studio.

## Licensing

Copyright (C) 2026 Andrew Arrow

Licensed under the GNU General Public License v3.0 or later.
See [LICENSE](LICENSE).
