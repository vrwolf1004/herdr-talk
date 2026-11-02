# Development

## Layout

```
herdr-talk/                      repository
├── .github/workflows/          release.yml builds binaries and signs checksums on a v* tag
├── docs/                       shared documentation
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
            ├── src/main.rs     command line, dock block
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

Run it in a real terminal. To test on one machine, start two instances with different names.

For herdr, run `bash scripts/build.sh` inside `rust/`, then link the `rust/` folder. `plugin link` skips the `[[build]]` step, so rebuild after a change, then close and reopen the pane.

## Wire format

Keep this stable: other PCs may run an older version, including the Python edition.

- Discovery, UDP broadcast: `{"app": "herdr-talk", "id", "name", "port", "key"}`. `id` is the SHA-256 of `key` for Rust peers. `key` is absent for Python peers.
- Message, one TCP connection each. Plain: `{"app": "herdr-talk", "id", "from", "text"}`. Encrypted: a Noise IK handshake, then one encrypted copy of the same JSON. See [configuration.md](configuration.md#encryption).

## Adding or changing a message

Every file in `rust/crates/talk-core/locales/` must have the same keys and placeholders. A test checks the keys. Use a message with `t("key", &[("name", value)])`.

## Adding a command

1. Add a branch to `Chat::handle_with()` in `chat.rs`.
2. Add a line to `help_box()` and a `help.*` key in every locale file.
3. Document it in both usage guides.

## Releasing

Set the same version in `rust/herdr-plugin.toml` and `rust/Cargo.toml`, then push the tag `v<version>`. The release workflow builds the binaries, signs the checksums, and publishes the release. The installers download the release matching the manifest version.

## Checks before a commit

- `cd rust && cargo test --workspace`
- `cd rust && cargo clippy --workspace --all-targets -- -D warnings`
- Run the chat once and check the changed screen or command.
