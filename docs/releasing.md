# Releasing Exaudio

Exaudio uses [Velopack](https://velopack.io): channel-specific installers, delta updates,
and in-app self-update. Release workflows target the planned public `Nominy/exaudio`
repository and use GitHub-hosted runners; they do not run on self-hosted machines.

The Babel integrations and browser extensions authored in the companion Babel checkout are
first-party developments. Their ownership is distinct from that of any files copied verbatim
from an upstream service. Review those external assets and runtime dependencies individually;
do not classify the entire Babel integration as third-party proprietary code.

Release and CI runners need no private build input: the Babel linter's rule data is tracked in
`extensions/babel-lint/assets`, and the private Babel data bundle (`tools/babel-data.json`) only
feeds the parity tooling and the self-skipping parity tests (see
[development.md](development.md#babel-data-bundle-parity-tooling-only)).

| Channel | Who gets it | Built by | Velopack channel | Feed |
| --- | --- | --- | --- | --- |
| Stable | everyone (default for release downloads) | `v*` tag → `release.yml` | one per platform: `win`, `osx-arm64`, `osx-x64`, `linux-x64`, `linux-arm64` | GitHub Releases of `Nominy/exaudio` (pre-releases excluded); each release carries every platform's `releases.<channel>.json` |
| Alpha | testers | every push to `main` → `alpha.yml` (Windows) and `linux-alpha.yml` (Linux x64, arm64); manual `macos-alpha.yml` | `alpha` | `https://reviewgen.ovh/exaudio/alpha/win-x64`, `.../linux-x64`, `.../linux-arm64`, `.../mac-arm64`, `.../mac-x64` |

Stable download endpoints (`https://github.com/Nominy/exaudio/releases/latest/download/<asset>`):

| Platform | Runner | Target | Asset |
| --- | --- | --- | --- |
| Windows x64 | `windows-latest` | `x86_64-pc-windows-msvc` | `Exaudio-win-Setup.exe` (installer), `Exaudio-win-Portable.zip` (portable) |
| macOS Apple silicon | `macos-15` | `aarch64-apple-darwin` | `Exaudio-mac-arm64.dmg` |
| macOS Intel | `macos-15-intel` | `x86_64-apple-darwin` | `Exaudio-mac-x64.dmg` |
| Linux x64 | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `Exaudio-linux-x64.AppImage` |
| Linux arm64 | `ubuntu-22.04-arm` | `aarch64-unknown-linux-gnu` | `Exaudio-linux-arm64.AppImage` |

Plus `SHA256SUMS` over every user-facing asset and update package. Linux builds are made on
Ubuntu 22.04 so the AppImage's glibc floor is 2.35; macOS builds target macOS 14.0.

## In-app updates

- A build follows the channel it was packaged for (`STUDIO_UPDATE_CHANNEL=stable|alpha`,
  baked in at compile time; plain `cargo build` means stable). Settings → Updates switches
  channel; the choice is stored in the UI storage and checked immediately.
- Installed builds check on launch and then at most every 6 hours, in the background. When an
  update exists, a small card in the dashboard sidebar offers **Install** (with download
  progress) or **Skip** (hides that version; Settings still offers it). Editing is never
  interrupted.
- Switching from Alpha to Stable never downgrades: the app moves to Stable with the first
  stable release newer than the installed alpha.
- Source builds (`cargo run`) are not Velopack installs; the Updates card says so and no
  check runs.

## Cut a stable release

1. Bump `version` in `crates/studio-app/Cargo.toml` (SemVer, e.g. `0.3.1`) and run any cargo
   command (`cargo check -p studio-app`) so `Cargo.lock` follows.
2. Commit and push to `main`; wait for CI.
3. Tag, push the tag, and start the release **from `main`** (a tag push by itself triggers
   nothing: Actions caches are scoped per ref, and only runs on `main` reuse the compiler
   caches of earlier runs — a cold build takes 3-4x longer):

   ```powershell
   git tag v0.3.1
   git push origin v0.3.1
   gh workflow run release.yml --ref main -f tag=v0.3.1
   ```

Builds use `Swatinem/rust-cache` for dependency artifacts plus `sccache` (GitHub Actions
cache backend) for compiled workspace crates; `sccache --show-stats` is printed per build job.

`release.yml` then runs three stages:

1. **prepare** (`ubuntu-latest`): fails unless the tag is exactly `v` + the `studio-app`
   version; generates the release notes (GitHub's generated notes plus the download and
   unsigned-package lines) once for every job.
2. **build** (one job per platform, see the matrix above; `fail-fast: false`):
   - builds `studio-app`, `studio-relay` and `exaudio-wasm` together in one Cargo invocation,
     keeping shared dependency features unified, with `STUDIO_UPDATE_CHANNEL=stable`;
     `exaudio-wasm-worker` is the isolated compiler child for untrusted extension `.wasm` and
     ships beside the main executable (`Exaudio.exe`, `Exaudio.app/Contents/MacOS/Exaudio`,
     the AppImage's `usr/bin/Exaudio`);
   - Linux installs the system packages from `.github/actions/linux-build-deps` (shared with
     CI); macOS imports the Developer ID certificate when the `APPLE_*` secrets exist;
   - downloads the previous release's full package for its channel (`vpk download github`)
     so the new release carries a delta package;
   - packs: Windows `tools/package-windows.ps1 -UpdateChannel stable` (`vpk pack`, pack id
     `Exaudio`, main exe `Exaudio.exe`, icon `assets/app/exaudio.ico`, channel `win`);
     macOS and Linux `UPDATE_CHANNEL=stable tools/package-unix.sh` (channel = Velopack
     runtime, bundle id `io.github.nominy.exaudio`, icon rendered from
     `assets/app/exaudio-512.png`; macOS wraps the Velopack-packed `Exaudio.app` into
     `Exaudio-mac-<arch>.dmg` with `hdiutil`, Linux takes Velopack's
     `Exaudio-linux-<arch>.AppImage`);
   - checks the expected assets exist and uploads `target/velopack/Publish` as the artifact
     `exaudio-<version>-<platform>`.
3. **publish** (`ubuntu-latest`): downloads every platform's payload, re-verifies each file
   against its `publish-manifest.json`, refuses duplicate asset names, writes one
   `SHA256SUMS` over every asset (installers, disk images, AppImages, full and delta packages)
   and keeps the assembled set as the artifact `exaudio-<version>-release`. Then it creates
   the GitHub release as a draft named `Exaudio <version>` (`--prerelease` for hyphenated
   versions), uploads everything with `gh release upload --clobber` and publishes it, so
   installed apps never see a half-uploaded release.

Release assets per platform: the user-facing asset(s) from the matrix,
`Exaudio-<version>[-<channel>]-full.nupkg`, `Exaudio-<version>[-<channel>]-delta.nupkg`
(from the second release on; the `win` channel has no suffix), the Velopack feed files
`releases.<channel>.json` (what installed apps read) and `RELEASES` / `RELEASES-<channel>`,
plus one `SHA256SUMS`.

A tag with a hyphen (`v0.4.0-rc.1`, version `0.4.0-rc.1` in Cargo.toml) becomes a GitHub
pre-release; stable installs ignore pre-releases.

To rebuild an existing tag, delete its GitHub release (draft or published) and run the
**Release** workflow manually with the tag as input. The workflow only runs in
`Nominy/exaudio`.

### Dry run

Run the **Release** workflow manually with `dry_run` checked on any branch (`tag` optionally
picks the ref to build): every platform builds and packs from that ref as version
`<major>.<minor>.<patch+1>-dry.<run number>.<run attempt>` — above the latest release, so
Velopack still produces delta packages against it, and below the next one — then `publish`
assembles `SHA256SUMS` and verifies it, and nothing touches GitHub Releases. The result is
the `exaudio-<version>-release` artifact (7 days). Use it after touching the packaging
scripts, the Linux dependency list or the workflow itself.

### Package locally

Needs the .NET 8 SDK and CMake (the bundled libopus is built from source; GitHub's runners
ship both C toolchains and CMake). Windows additionally the MSVC toolchain:

```powershell
dotnet tool restore
.\tools\package-windows.ps1 -UpdateChannel stable
```

macOS (Xcode command-line tools) and Linux (the packages from
`.github/actions/linux-build-deps/action.yml`):

```bash
dotnet tool restore
UPDATE_CHANNEL=stable tools/package-unix.sh
```

`target/velopack/Releases` receives the full local feed (installer or disk image or
AppImage, packages, feed metadata); `target/velopack/Publish` the current release's assets
plus `publish-manifest.json` (byte size and SHA-256 of each file). Windows: `-Version`,
`-ReleaseNotes`, `-OutputDir`, `-PublishDir`, `-SkipBuild` are optional. macOS/Linux: the
`VERSION`, `RUNTIME`, `RELEASE_NOTES`, `STAGE_DIR`, `OUTPUT_DIR`, `PUBLISH_DIR` and
`MACOSX_DEPLOYMENT_TARGET` environment variables are optional; the script never
cross-compiles (`RUNTIME` must match the host).

Stable Windows feed metadata includes `RELEASES`, `assets.win.json` and `releases.win.json`.
Non-default channels use the suffixed legacy index, for example `RELEASES-osx-arm64` or
`RELEASES-alpha`. `assets.<channel>.json` is a packaging aid and is not uploaded.

## Alpha

Every push to `main` runs `alpha.yml` on `windows-latest` (one build per push, runs
serialized, never cancelled):

- version `<major>.<minor>.<patch+1>-alpha.<run number>.<run attempt>`, which sorts above the
  current stable and below the next one; it is compiled in as `STUDIO_BUILD_VERSION`;
- the previous alpha is downloaded from the feed for delta packages;
- `tools/package-windows.ps1 -UpdateChannel alpha` packs channel `alpha`
  (`Exaudio-alpha-Setup.exe` is the tester installer);
- `tools/publish-alpha-windows.ps1 -SkipPackage` publishes `target/velopack/Publish` to the
  VPS (`/var/www/exaudio/alpha/win-x64`, served as `https://reviewgen.ovh/exaudio/alpha/win-x64`):
  upload to an unserved incoming directory (`/var/tmp/exaudio-alpha-incoming`), verify every
  file against the manifest under a remote lock, promote packages before
  `releases.alpha.json`, keep the previous feed metadata for rollback. Versioned packages are
  immutable on the server.

Push runs always publish (and fail if the secrets are missing); a manual dispatch publishes
only with `publish_to_vps: true`. Each run also keeps the payload as a 3-day artifact.

Publish from a developer machine with an SSH alias for the server:

```powershell
.\tools\publish-alpha-windows.ps1 -Remote <ssh-alias>
```

It packages first unless `-SkipPackage` is given.

### One-time reinstall after the rename

Up to 0.2.x the private alpha was packaged as `TranscriptionStudio` and published to
`https://reviewgen.ovh/transcription-studio/alpha/...`. Exaudio is a new Velopack package
(`Exaudio`, installed into `%LocalAppData%\Exaudio`) on its own feeds under
`https://reviewgen.ovh/exaudio/alpha/...`; nothing publishes to the old feeds any more, so old
installs stay frozen and never see an Exaudio package. Install Exaudio once from
`Exaudio-alpha-Setup.exe` (or a stable release), then uninstall the old `TranscriptionStudio`
package. Sign-in, drafts and settings are not carried over: Exaudio reads its application-support
directory `Exaudio`, not the old `Transcription Studio` one.

Before the first Exaudio alpha publish, the web server must serve `/var/www/exaudio/` as
`https://reviewgen.ovh/exaudio/` (the publisher creates the feed directories itself).

### macOS alpha

`macos-alpha.yml` is manual (`workflow_dispatch`). It builds natively on GitHub-hosted macOS
runners with `UPDATE_CHANNEL=alpha tools/package-unix.sh` (pack id `Exaudio`, bundle id
`io.github.nominy.exaudio`, channel `alpha`, deployment target macOS 14.0; the alpha asset is
Velopack's `Exaudio-alpha-Portable.zip`) and can publish to:

| Target | Runner | Runtime | Feed |
| --- | --- | --- | --- |
| `latest-arm64` | `macos-15` | `osx-arm64` | `https://reviewgen.ovh/exaudio/alpha/mac-arm64` |
| `latest-amd64` | `macos-15-intel` | `osx-x64` | `https://reviewgen.ovh/exaudio/alpha/mac-x64` |
| `sonoma-amd64` | `macos-14-large` | `osx-x64` | `https://reviewgen.ovh/exaudio/alpha/mac-sonoma-x64` |

`STUDIO_ALPHA_FEED_URL` bakes each target's feed into its build. `macos-14-large` is a
larger runner and is billed even for public repositories; skip `sonoma-amd64` unless that is
intended. Publishing goes through `tools/publish-alpha-unix.sh` (the same incoming-directory,
verify, promote-metadata-last and rollback steps as the Windows publisher).

### Linux alpha

Every push to `main` also runs `linux-alpha.yml` (serialized, never cancelled), one job per
architecture:

| Runner | Runtime | Asset | Feed |
| --- | --- | --- | --- |
| `ubuntu-22.04` | `linux-x64` | `Exaudio-alpha.AppImage` | `https://reviewgen.ovh/exaudio/alpha/linux-x64` |
| `ubuntu-22.04-arm` | `linux-arm64` | `Exaudio-alpha.AppImage` | `https://reviewgen.ovh/exaudio/alpha/linux-arm64` |

Each job uses the alpha version scheme above (with its own run number), downloads the
previous alpha from its feed for delta packages, packs with
`UPDATE_CHANNEL=alpha tools/package-unix.sh` and publishes `target/velopack/Publish` with
`tools/publish-alpha-unix.sh` to `/var/www/exaudio/alpha/linux-<arch>`. Push runs always
publish; a manual dispatch publishes only with `publish_to_vps: true`. The AppImage needs
`chmod +x` once; it then updates itself from the feed.

## Secrets

| Secret | Used by | Required |
| --- | --- | --- |
| `ALPHA_SSH_HOST` | `alpha.yml`, `linux-alpha.yml`, `macos-alpha.yml` publish | yes, for publishing |
| `ALPHA_SSH_USER` | same | yes, for publishing |
| `ALPHA_SSH_PRIVATE_KEY` | same; deploy key with write access to the incoming and feed directories | yes, for publishing |
| `ALPHA_SSH_KNOWN_HOSTS` | same; the server's `known_hosts` line(s), e.g. from `ssh-keyscan -p <port> <host>` checked against the server | yes, for publishing |
| `ALPHA_SSH_PORT` | same | no (default 22) |
| `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_KEYCHAIN_PASSWORD`, `APPLE_DEVELOPER_ID_APPLICATION`, `APPLE_DEVELOPER_ID_INSTALLER`, `APPLE_NOTARY_PROFILE`, `APPLE_NOTARY_APPLE_ID`, `APPLE_NOTARY_TEAM_ID`, `APPLE_NOTARY_PASSWORD` | `release.yml` and `macos-alpha.yml` macOS signing/notarization | no (unsigned without them) |

Beyond the optional Apple secrets, `release.yml` needs none: it uses the workflow's
`GITHUB_TOKEN` (`contents: write`).

## Signing

- **Windows: unsigned.** SmartScreen shows "Windows protected your PC" on first launch of the
  installer or portable exe until the download builds reputation; users choose
  **More info → Run anyway**. Every release lists SHA-256 checksums in `SHA256SUMS`.
- **macOS: optional.** With the Apple secrets the workflows sign with Developer ID and
  notarize; without them the disk image (stable) and portable bundle (alpha) are unsigned and
  there is no `.pkg`. Users open an unsigned build once via **System Settings → Privacy &
  Security → Open Anyway**.
- **Linux: unsigned.** The AppImage must be made executable (`chmod +x`) and needs FUSE 2
  (`libfuse2`) on most distributions; `SHA256SUMS` is the integrity check.

## CI

`ci.yml` runs on every push and pull request. On `windows-latest` (the full job): gitleaks
over the tree and the full history (`tools/scan-secrets.ps1`), `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo nextest run --workspace --all-targets --locked`. On `macos-15` and `ubuntu-22.04`
(the Linux job installs `.github/actions/linux-build-deps` first): the same clippy and
nextest runs, so platform gates (`cfg(windows)` tray, WebView2 local inference host,
relay/transport spots) stay compiling and tested everywhere. The `.githooks/pre-push` hook
runs the fmt and clippy gate locally (`git config core.hooksPath .githooks`).

## Local inference runtime

"This PC" AI runs a separately published inference runtime. It is not part of the installer
or of this repository: `extensions/gold/gold-inference.json` pins exactly one
published build (version, source commit, URL, SHA-256, size, extension id). Exaudio downloads
it on first local use (or Settings → Install), verifies size and SHA-256 while streaming and
extracts it atomically into its data directory; the active and the previous build are kept.

Prerequisites for shipping a pin in a release:

- the pinned URL is public and immutable (a re-upload must be byte-identical), so every
  released build keeps downloading exactly what it pins;
- the runtime's licence allows downloading it onto users' machines from Exaudio;
- changing the pin is a reviewed change to `gold-inference.json`, made by a maintainer with
  access to the runtime's source; the end-to-end check is the `local_smoke` example of
  `exaudio-gold` (`extensions/gold`) on a WebGPU machine.
