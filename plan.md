# Desktop Player direction

Desktop is the focused native game runtime for macOS, Windows, and Linux. The
website is the platform control plane while the desktop app is young.

```text
cubacadabra.com
├── Cube discovery
├── account and username
├── Morph/avatar editing
├── safety and blocked players
├── subscriptions
└── Play
      ↓
Cubacadabra Desktop
├── launch/loading/errors
├── native input and rendering
├── multiplayer session
├── in-game return/home
└── resume or choose another Cube
```

This is a deliberate product boundary, not a consequence of browser-based
login. Browser login is only the authentication ceremony. After its localhost
callback and code exchange, Desktop owns the authenticated runtime session and
uses its access token for the game WebSocket.

## Native Desktop responsibilities

- Load and validate an installed game package.
- Render and run the shared Rust game client.
- Connect authenticated multiplayer sessions.
- Present launch, loading, connection, and package errors.
- Return from a game to a small native player menu.
- Resume the current Cube without a browser round trip.
- Open the web catalog, account, and informational surfaces.
- Eventually accept validated Cube launch deep links.
- Keep basic runtime settings that directly affect native play.

## Web control-plane responsibilities

- Browse and discover Cubes.
- Username and account details.
- Morph/avatar selection.
- Blocked players and safety controls.
- Password, email, verification, and recovery.
- Parent administration.
- Subscription and billing.
- Legal, privacy, and support.
- Creator and administrative tools.

These flows should only move native after their shared `cubacadabra-app`
effects, backend persistence, loading, error, and conflict behavior are
complete. Desktop should not present account controls that only mutate local
session state.

## Current native menu

```text
CUBACADABRA

[ Continue playing ]
[ Browse games on cubacadabra.com ]

Signed out:
[ Sign in ]

Signed in:
Signed in as <username>
[ Account & avatar on cubacadabra.com ]

[ About cubacadabra ]
```

Debug builds open the local web app and backend:

```text
Web:     http://127.0.0.1:5173
Backend: http://127.0.0.1:8787
```

Release builds open production:

```text
Web:     https://cubacadabra.com
Backend: https://api.cubacadabra.com
```

`CUBACADABRA_WEB_URL` and `CUBACADABRA_BACKEND_URL` remain explicit runtime
overrides.

## Authentication

```text
Desktop
   ↓ opens /login/
system browser
   ↓ authenticated redirect
localhost callback with state + code
   ↓ code exchange
Desktop access/refresh token session
   ↓
authenticated game WebSocket
```

Before production distribution, authentication still needs:

- OS credential storage instead of memory-only tokens.
- Access-token expiry handling and refresh-token rotation.
- Logout and credential deletion.
- Clear expired-session recovery.
- Tests for callback validation and refresh failure.

## Next platform step: launch deep links

The website should eventually launch Desktop with a narrow protocol such as:

```text
cubacadabra://play/<cube-id>
```

The handler must:

- accept only the `play` action;
- validate and length-limit the Cube identifier;
- reject arbitrary URLs, paths, query-driven commands, and credentials;
- resolve package metadata through the configured Cubacadabra service;
- verify downloaded package integrity before launch;
- show a native confirmation/error surface when launch cannot continue;
- support macOS, Windows, and Linux registration;
- let the website fall back to install/download instructions.

Until that protocol and package retrieval path are ready, Desktop starts from
the bundled `first-game` package and accepts a package path as a development
override; the web catalog opens in the browser.

## Studio boundary

Studio remains a separate creator product. Desktop and Studio can share Rust
engine, renderer, networking, authentication, and OS-integration utilities,
but they should not share a product shell.

```text
Shared Rust/runtime infrastructure
            │
      ┌─────┴─────┐
      │           │
   Studio      Desktop
 creator UI   player runtime
```

The architecture can grow toward a fuller native client later. That should be
driven by player value and complete shared flows, not by duplicating web pages
in egui for architectural symmetry.
