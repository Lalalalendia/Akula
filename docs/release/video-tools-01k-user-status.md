# VIDEO-TOOLS-01k — protected status in the installed Akula GUI

**Scope:** read-only Video Tools status and explicit Download refusal in future installed Windows builds. No publishing, update or user deployment.

### What changes

Clicking the Download control on a YouTube card in a normal Windows **installed** build opens a localized (Russian or English) notice based on native `probe_installed_video_tools()`, never on `PATH`, `AKULA_YT_DLP_BIN` or a user-selected directory:

| Verified diagnostic state | Meaning to the user | Can enqueue download? |
| --- | --- | --- |
| `Missing` | Separate Program Files product absent; manual official installation after release | **No** |
| `Unsafe` | Root/ACL/contents/signature could not pass required native checks | **No** |
| `PublisherNotApproved` | Product present, owner has not approved its signing key and shipping gate | **No** |
| `IncompatibleVersion` | Authenticated manifest is not the candidate V1 `0.1.0` contract | **No** |
| `ReadOnlyAttested` | Native read-only signature/hash diagnostics pass, **not** executable admission | **No** |

The distinction between `PublisherNotApproved` and `Unsafe` matters: when there is no approved publisher key, native discovery deliberately stops after protected-root/ACL validation. It **does not** claim the test CI signature authenticates a real publisher. `IncompatibleVersion` can only be reached after owner-key authentication. The exact `0.1.0` compatibility policy must be revisited with the first actual publisher-approved package.

The ordinary installed UI's Download click **never queues a process** until there is a separately reviewed executable admission path. The queue also fails closed **before** writing job state or starting a thread, and `download_one_or_legacy` refuses to run the legacy PATH/variable-selected yt-dlp in installed Windows builds. Only the explicitly **nonshipping** `video-ui-e2e` feature retains its existing laboratory path. Developer/non-installed builds keep their existing direct downloader behavior.

Link preview, URL text, and reading already saved device-local video do not invoke these guards and remain available. The stock Akula two-EXE installer and its updater remain separate; no real Video Tools install or download link is offered before release.

### Acceptance

- Native Windows `--features installed` compilation and targeted tests for all five localized states and for queue refusal without creating a job or local bytes.
- Source mutation tests ensure the UI cannot rely on PATH or environment-selected executables, and the queue/leaf launch guards cannot be removed.
- Existing VIDEO-TOOLS-01j signed six-file install, protected ACL, real Minisign, five negative controls, uninstall and foreign-note preservation remain passing.
- `APPROVED_VIDEO_TOOLS_KEY=""`, `shipping_admission_enabled()=false`, `VIDEO_FIVE_CHANNEL_READY=false` and **VIDEO-18 RELEASE HOLD** remain untouched.

### Not completed by 01k

There is **no** approved end-user Video Tools release, owner signing chain, installer version-update UX, GUI screenshot acceptance, or real video execution through pinned owner-authenticated handles. Showing `ReadOnlyAttested` in the dialog is not permission to execute a runtime. A separately reviewed phase must enable and test the actual process handoff.
