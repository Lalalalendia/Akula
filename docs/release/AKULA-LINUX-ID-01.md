# RELEASE-LINUX-ID-01 — independent Linux DEB/RPM Akula candidates

Status: unsigned temporary CI packages, not a public release.
Linux Aquilum owns package aquilum, binary /usr/bin/aquilum-app,
Aquilum.desktop and data directory com.dmitriy.aquilum-app.
Akula Linux now uses separate AppData ID io.github.lalalendia.akula,
window title Akula and autostart Akula.desktop. Linux automatically
importing Aquilum data is disabled. macOS is unchanged/frozen.
New nfpm config builds package akula, executable /usr/bin/akula-app,
Akula.desktop and akula-app icons. Original Aquilum package files remain.
CI builds locked Rust, checks pinned nfpm SHA256, installs DEB on disposable
Ubuntu, runs Xvfb/Openbox installed GUI, verifies synthetic note bytes,
AppData and uninstall, then tests Fedora RPM install/remove. Upstream
Aquilum binary, desktop and data sentinels must survive with identical hashes.
AppImage, real Linux physical QA, signing, public release, updater,
Akula-to-Akula rollback, power-loss and S3 apply are still on HOLD.
Related PR #437 release lock, PR #366 Windows provenance.
Master: https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b