# Development

## Layout

```
herdr-talk/                      repository
├── .github/workflows/          release.yml builds binaries and signs checksums on a v* tag
├── docs/                       shared documentation
├── gui/                        Windows GUI (own crate, not in the rust/ workspace)
│   ├── Cargo.toml              eframe, talk-core by path
│   ├── src/main.rs             window, control-channel client, embedded mode, themes, CRT look
│   └── assets/                 Galmuri11.ttf (SIL OFL) and its license
├── python/                     Python edition (plugin folder, no build)
│   ├── herdr-plugin.toml
│   └── src/talk.py
└── rust/                       Rust edition (plugin folder)
    ├── herdr-plugin.toml       herdr manifest
    ├── Cargo.toml              workspace
    ├── herdr/install.sh, .ps1  download the release binary into bin/ at install, or build from source
    ├── scripts/build.sh        build into bin/ for a linked local copy
    └── crates/
        ├── talk-core/          no herdr or terminal code
        │   ├── src/net.rs      discovery, messages, encryption
        │   ├── src/chat.rs     shared state, commands, names, help box
        │   ├── src/store.rs    identity key, verified peers, fingerprint
        │   ├── src/text.rs     cleaning, width, wrapping
        │   ├── src/i18n.rs     language choice and t(key, values)
        │   └── locales/<language>.json
        └── herdr-talk/         the plugin binary
            ├── src/main.rs     command line (run, serve, open, dock), dock block
            ├── src/control.rs  local control channel (lines, peers, send, shutdown)
            ├── src/screen.rs   full-screen chat
            ├── src/dock.rs     dock pane allocation
            └── src/herdr_api.rs herdr CLI and socket
```

The plan beyond this is in [rust-roadmap.md](rust-roadmap.md). Paths below are relative to `rust/` unless they start with the repository root.

## Running locally

```sh
cd rust
cargo run -p herdr-talk -- run --name test
```

Run it in a real terminal. To test on one machine, start two instances with different names. The same name also works: the node with the larger id adds a number.

Headless, and the control channel (the port and token are in `control-<pid>.json` in the state folder):

```sh
cargo run -p herdr-talk -- serve --name test
```

The GUI is a separate crate. Build `herdr-talk` first so the GUI finds `rust/target/release/herdr-talk.exe`, then:

```sh
cd gui
cargo run --release
```

Cargo cannot overwrite a running exe on Windows. Close the GUI and any `serve` before rebuilding, or use another `CARGO_TARGET_DIR`. The GUI does not build with the `rust/` workspace, so run its checks inside `gui/` too.

For herdr, run `bash scripts/build.sh` inside `rust/`, then link the `rust/` folder. `plugin link` skips the `[[build]]` step, so rebuild after a change, then close and reopen the pane.

## Wire format

Keep this stable: other PCs may run an older version, including the Python edition.

- Discovery, UDP broadcast: `{"app": "herdr-talk", "id", "name", "port", "key"}`. `id` is the SHA-256 of `key` for Rust peers. `key` is absent for Python peers.
- Message, one TCP connection each. Plain: `{"app": "herdr-talk", "id", "from", "text"}`. Encrypted: a Noise IK handshake, then one encrypted copy of the same JSON. See [configuration.md](configuration.md#encryption).

## Name clashes

`Chat::resolve_name_conflict()` in `chat.rs` is called when a peer is remembered (`net.rs`), before the first announcement (after `NAME_SETTLE`), and after `/name` and `/lang`. Rules: only verified peers (key hashes to id) count, the larger id renames (port breaks ties between equal ids), the new name is the first 30 characters plus the lowest free number from 2, and an automatic rename happens at most once per `AUTO_RENAME_GAP` (60 s). `/name` and `/lang` bypass the limit. The Python edition (`resolve_name_clash` in `talk.py`) uses the same numbering and limit, without verification. Keep both editions' rules and tests in step.

## Adding or changing a message

Every file in `rust/crates/talk-core/locales/` must have the same keys and placeholders. A test checks the keys. Use a message with `t("key", &[("name", value)])`.

## Adding a command

1. Add a branch to `Chat::handle_with()` in `chat.rs`.
2. Add a line to `help_box()` and a `help.*` key in every locale file.
3. Document it in both usage guides.

## Releasing

Set the same version in `rust/herdr-plugin.toml` and `rust/Cargo.toml` (currently 0.7.0; the GUI has its own version and is not part of the release), then push the tag `v<version>`. The release workflow builds the binaries, signs the checksums when the `MINISIGN_SECRET_KEY` secret exists in the `release` environment, and publishes the release. Without the secret, the release is published unsigned and the installers check only the SHA-256. The installers download the release matching the manifest version.

## Checks before a commit

- `cd rust && cargo test --workspace`
- `cd rust && cargo clippy --workspace --all-targets -- -D warnings`
- `cd python && python -m unittest discover -s tests`
- GUI changes: `cd gui && cargo build --release`, then open the window once (start screen, theme select, CRT checkbox)
- Run the chat once and check the changed screen or command.
