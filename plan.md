I’d make the desktop Player a **real fourth client**, not a thin game window that punts its core shell to the website.

The clean model is:

```text
                         cubacadabra-app
                    shared semantic app state
              account / catalog / avatar / safety
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                   │
         Web                Mobile              Desktop
          │              iOS + Android       macOS/Win/Linux
          │                   │                   │
         DOM           SwiftUI / Compose         egui
```

You already built `cubacadabra-app` specifically so username/profile/catalog/safety behavior does not get independently reimplemented by every host. The host owns presentation and OS integration while Rust owns the semantic state machine.  Desktop Player is almost the perfect additional consumer of that architecture.

So I would have the desktop app own these **inside the app**:

* home / game discovery and “continue playing”
* selecting and launching a Cube
* account summary
* changing username/profile basics
* avatar/Morph selection and 3D preview
* blocked users/basic safety
* app settings
* login/logout state
* package download/update state
* game launch/loading/error UX

And implement that desktop shell in Rust/egui, backed by `cubacadabra-app` and `cubacadabra-client`.

### Where I would still send users to the web

There is no need for ideological purity. Some uncommon or browser-native flows can initially leave the app:

```text
Desktop Player                         Website

browse/play games            ✓
avatar                       ✓
username                     ✓
settings                     ✓
normal account management    ✓

email verification                    → web
password recovery                      → web
legal/privacy pages                    → web
subscription checkout                  → web, where permitted
creator dashboards                     → web
parent administration                  → probably web initially
deep billing history                   → web
unusual support/admin flows            → web
```

That is common desktop-app behavior and doesn't make the app feel incomplete.

The test I'd use is:

> **Does a normal player expect to do this while deciding what to play or how they appear in games?**

If yes, it belongs in the desktop Player.

Having to launch a browser merely to change your avatar or pick another game would feel noticeably cheap.

---

There's another architectural reason I like this: desktop becomes your **cleanest host**.

Unlike iOS and Android, desktop Player can consume your Rust crates directly:

```text
cubacadabra-player
    │
    ├── cubacadabra-app
    ├── cubacadabra-client
    ├── cubacadabra-engine
    └── renderer
```

No:

```text
C ABI
JNI
Swift bridge
WASM
JavaScript adapter
```

So:

```text
macOS / Windows / Linux
            │
           egui
            │
     cubacadabra-app
            │
     cubacadabra-client
            │
    cubacadabra-engine
```

That should arguably become the **reference implementation of a complete native client**.

Studio would share plenty of lower-level infrastructure but remain a different product:

```text
                     Shared Rust
                         │
           ┌─────────────┴─────────────┐
           │                           │
        Studio                       Player
           │                           │
     creator shell                 player shell
     project editing               catalog/home
     asset import                  account/avatar
     test sessions                 game launcher
     Codex                         social/safety
           │                           │
 macOS / Win / Linux          macOS / Win / Linux
```

I would **not** try to turn Studio itself into the desktop Player.

### Login can still use the browser

This is one place where sending someone out to the browser is totally fine.

Something like:

```text
Desktop Player
      │
      │ Sign in
      ▼
system browser
      │
Cubacadabra / Google / whatever
      │
 loopback callback / app link
      ▼
Desktop Player authenticated
```

Studio already has a browser-based authentication pattern conceptually. The desktop Player can use the same general host-owned auth approach; the shared Rust app state shouldn't become an OAuth implementation.

### One caution about egui

I would use egui, but don't make:

> “everything must be custom-drawn because Rust”

a rule.

For desktop OS-owned things, still use native integration:

* file/open dialogs if ever needed
* URL launching
* clipboard
* notifications
* accessibility hooks
* system menus where appropriate
* credential/keychain storage
* browser login
* window lifecycle

The actual **Cubacadabra application UI** can be shared egui.

That is essentially the same boundary you've already chosen for Studio.

---

So I’d update the architecture mentally to four player clients:

```text
PLAYER CLIENTS

1. Web
   DOM + WASM

2. iOS
   SwiftUI + C/Rust

3. Android
   Compose + JNI/Rust

4. Desktop
   egui + direct Rust
   ├── macOS
   ├── Windows
   └── Linux
```

Not six separate clients. **One desktop client with three OS targets.**

And then:

```text
CREATOR CLIENT

Cubacadabra Studio
   egui + direct Rust
   ├── macOS
   ├── Windows
   └── Linux
```

That distinction makes the entire product architecture much clearer.

I would only punt peripheral account-management complexity to the website. **Game selection, avatar, username, settings, and the normal player shell should absolutely be first-class desktop UI.**

