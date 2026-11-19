# VIDEO-TOOLS-01l: real Windows GUI denial and package-state diagnostics

Scope: non-release GitHub-hosted Windows acceptance of ordinary installed Akula
download refusal. No production release, publisher signing or executable
admission is enabled.

The Akula candidate has feature installed plus the observation-only feature
video-tools-gui-e2e. The latter does NOT imply video-ui-e2e or video-rust-bundle,
and cannot enable video downloading. It only records bounded real OS pointer
events and prevents external YouTube thumbnail fetches in this test.

The real Windows GUI acceptance:

1. Creates a fresh temporary Markdown vault and device-only data folder.
2. Locates native editor via Windows UI Automation, uses Akula's hover-hit
   coordinates for the Download toolbar and sends actual foreground-PID-checked
   Win32 SendInput mouse down/up.
3. Requires complete exact native pointer, Download, and NOTICE_BLOCKED
   records, and the matching visible AlertDialog title inside the Akula HWND.
4. Tests Missing (including ProgramFiles environment spoof),
   PublisherNotApproved (inert six-file admin/System-owned protected product),
   and Unsafe (an ordinary Users Modify ACE on one child).
5. Places an executable yt-dlp.exe sentinel at the selected environment
   path and first PATH directory; if it launches, a marker fails acceptance.
   Verifies no media is created, no video enters the synced vault, and
   original Markdown SHA-256 remains unchanged.
6. Removes only its own protected six-file test root after exact identity
   checking and refuses to touch a preexisting installation.

ReadOnlyAttested and IncompatibleVersion cannot be reached in the real
ordinary installed build while the approved publisher key is empty and
shipping admission false. Earlier bilingual tests cover their state mapping.
CI must not fake an approved publisher key as shipping authorization.

This is proof of installed-flavor GUI behavior on an ephemeral Windows runner,
NOT a signed installer or permission to ship. VIDEO-TOOLS-01j separately tests
temporary signed NSIS/ACL installation. VIDEO-18 and Video Tools remain under
RELEASE HOLD.
