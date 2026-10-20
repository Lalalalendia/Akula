# RELEASE-APPIMAGE-01 — independent, unsigned Akula Linux AppImage

Status: Draft CI-only candidate; NOT a signed or published release.

Depends on accepted RELEASE-LINUX-ID-01 [PR #439](https://github.com/Lalalendia/Akula/pull/439): Linux AppData ID io.github.lalalendia.akula and automatic Aquilum legacy import disabled.

The existing Cargo.toml [package.metadata.packager] intentionally retains Aquilum fields for historical compatibility and must never be used for Akula packaging. The independent `native/packaging/akula-linux.json` explicitly declares Akula productName, binary akula-app, icon resources, identifier io.github.lalalendia.akula, and immutable-git-SHA gstreamer linuxdeploy plugin URL.

CI compiles cargo build --release --locked --features installed, renames ONLY a copied binary, packages AppImage with cargo-packager v0.11.8 and explicit --config, and uses --appimage-extract to assert the actual embedded AppRun, Akula executable and Akula desktop entry. It runs the resulting AppImage with APPIMAGE_EXTRACT_AND_RUN=1 under Xvfb/Openbox in disposable Ubuntu 22.04 Docker, checks GUI opening/closing, synthetic note SHA-256, dedicated Akula AppData and unchanged mock Aquilum binary/desktop/userdata witnesses.

All evidence is produced for the exact GitHub Actions checkout commit. Both commit SHA and AppImage SHA-256 are attached to a 14-day ephemeral workflow artifact. SHA-256 is not proof of publisher identity.

Explicit exclusions: no `gh release`, updater key, private signing key, AppImage zsync updater metadata, release.yml publishing, real-system install, machine migration, physical Linux/Windows acceptance, macOS unfreeze or destructive S3/Sync apply. A separate signed publisher release and Akula-to-Akula rollback plan remains required.

Sources: [Akula master](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b); cargo-packager 0.11.8 config supports explicit --config (docs.rs/cargo-packager/0.11.8); AppImage extraction and linuxdeploy desktop embedding (docs.appimage.org).