<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/header-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/brand/header-light.svg">
  <img alt="Exaudio" src="assets/brand/header-light.svg" width="435">
</picture>

# From audio, every word.

A segment-based transcription editor for Windows, macOS and Linux: listen, fix timing, assign speakers and edit the text in one native window.
Local projects need no account; AI, checks and formats are extensions you can switch off.

**English** · [Русский](README.ru.md) · [Website](https://nafty.is-a.dev/exaudio/) · [Get started](docs/getting-started.md) · [Releases](https://github.com/Nominy/exaudio/releases) · [Issues](https://github.com/Nominy/exaudio/issues)

![Exaudio editor: a synthetic interview after an AI draft — waveform timeline and 11 segments with speaker and timing](site/assets/editor-interview-draft.webp)

## Download

| Platform | Stable (GitHub Releases) |
| --- | --- |
| Windows x64, installer | [Exaudio-win-Setup.exe](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Setup.exe) |
| Windows x64, portable archive | [Exaudio-win-Portable.zip](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip) |
| macOS, Apple silicon | [Exaudio-mac-arm64.dmg](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-mac-arm64.dmg) |
| macOS, Intel | [Exaudio-mac-x64.dmg](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-mac-x64.dmg) |
| Linux x64 | [Exaudio-linux-x64.AppImage](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-linux-x64.AppImage) |
| Linux arm64 | [Exaudio-linux-arm64.AppImage](https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-linux-arm64.AppImage) |
| Checksums | [SHA256SUMS](https://github.com/Nominy/exaudio/releases/latest/download/SHA256SUMS) |
| Windows alpha installer (separate feed) | [Exaudio-alpha-Setup.exe](https://reviewgen.ovh/exaudio/alpha/win-x64/Exaudio-alpha-Setup.exe) |

Stable releases are `v*` tags on GitHub Releases; every release carries all platforms and a `SHA256SUMS` file, and installed copies update themselves. Alpha comes from a separate VPS feed (Windows and macOS), not the stable update channel; back up projects before testing it. See [distribution and updates](docs/releasing.md).

**Packages are unsigned.** Windows SmartScreen may warn about an unknown publisher (More info → Run anyway). macOS cannot verify the app the first time: open **System Settings → Privacy & Security** and click **Open Anyway**. The Linux AppImage needs `chmod +x Exaudio-linux-x64.AppImage` (and FUSE 2, `libfuse2`, on most distributions). Download only from this project's Releases, compare the `SHA256SUMS` entry, and do not disable OS protection.

Requirements: Windows 10/11 x64, macOS 14+ or a 64-bit Linux desktop (X11 or Wayland, PulseAudio/ALSA), an audio output and writable space for projects. Local AI ("This PC") runs on Windows only: it additionally needs WebView2, a WebGPU-capable GPU with current drivers, and separately downloaded models (their own licenses; size, RAM/VRAM and disk depend on the model); on macOS and Linux use the remote server backend. Internet is needed for downloads, updates and remote services, not for editing local recordings.

## Features

- **Segments & speakers.** Each line has a speaker, start and end. Split at the playhead, merge neighbours, trim to audio, rename or add speakers from the row menu.
- **Timeline & waveform.** One lane per audio file, zoomable waveforms, an overview strip, playback speed, loop, undo/redo. Drag on the waveform to create a segment.
- **Local AI.** Align words, draft a transcript, fill gaps and denoise — on your GPU through WebView2/WebGPU, or on a remote server. Every result is a draft you review.
- **Checks.** A Checks column flags empty text, overlaps on the same track and stray spaces, with safe one-click fixes. Extensions add their own rules.
- **Import / export.** SRT, WebVTT and plain text in and out through the Formats extension (**Commands → Import transcript…** / **Export…**). The native project folder keeps a copy of the audio and every segment.
- **Extensions.** Commands, panels, checks, formats and modes come from extensions: bundled ones are native Rust, installable ones are sandboxed WASM.
- **Command palette (Ctrl+Shift+P).** Every command the editor and its extensions offer, searchable by name, with its shortcut beside it.

Start shows one tile per installed mode. **Local** opens audio and projects on this computer without an account. **Babel**, **Babel error review** and **Babel file** are optional modes supplied by the author's Babel integration; they need their own service access and are not needed for Local. Exaudio is not an official Babel product and does not claim Babel endorsement.

## Quick start

1. Run the installer or extract the portable archive (Windows), drag `Exaudio.app` to Applications (macOS), or `chmod +x` and run the AppImage (Linux).
2. On Start, choose **Local → Open audio…**. Each file becomes a lane; WAV, MP3 and FLAC are good starting points (OGG, M4A, MP4, WebM and MKV containers also open, depending on the codec inside).
3. Listen and edit: drag on the waveform to add a segment, `S` splits at the playhead, `D` deletes, `Space` plays, `←`/`→` move between segments, `Ctrl+Z`/`Ctrl+Y` undo/redo, `Ctrl+F` finds. Open the shortcuts dialog for the full, rebindable list.
4. Optional AI: in the AI extension pick **This PC** (install the runtime and models, run the device test) or the remote server, then use **Align words**, **Draft with AI** or **Denoise** and check the result before keeping it.
5. **Save** the native project (a folder with `project.json` and `audio/`), then **Export…** SRT, WebVTT or TXT when you need a delivery file. Unsaved-work snapshots, settings and extensions live under `%APPDATA%\Exaudio` (`~/Library/Application Support/Exaudio` on macOS, `~/.config/Exaudio` on Linux).

Step by step: [Getting started](docs/getting-started.md).

## Extensions

Installable extensions are `.exaudio` packages (WebAssembly components) built with the [Rust SDK](sdk/rust/README.md) against the typed [WIT API](wit/exaudio-extension.wit). They contribute commands, panels, checks, formats and modes, and access documents, audio, storage and network hosts only through explicit grants. Bundled native extensions run with the application's privileges.

The public catalog lives in [`Nominy/exaudio-extensions`](https://github.com/Nominy/exaudio-extensions) ([`index.json`](https://raw.githubusercontent.com/Nominy/exaudio-extensions/main/index.json)). Authors keep source and versioned packages in their own repositories; the index lists releases, compatibility, digests and requested permissions. Review the author, source and permissions before installing — a catalog entry is not a security endorsement.

## Build from source

Every platform needs Git, the Rust toolchain pinned in `rust-toolchain.toml` and CMake (the bundled libopus is built from source). Windows adds MSVC C++ build tools with the Windows SDK; macOS the Xcode command-line tools; Linux a C compiler plus the development packages listed in `.github/actions/linux-build-deps/action.yml` (xkbcommon, Wayland, X11/XCB, Vulkan, fontconfig, ALSA, D-Bus). Internal crates keep the `studio-*` prefix.

```powershell
git clone https://github.com/Nominy/exaudio.git
cd exaudio
cargo run -p studio-app
```

The default build includes Local, Formats, basic checks, the Gold AI tools and the Babel integration, and builds from a clean clone. Feature subsets:

```powershell
# AI tools, no Babel
cargo run -p studio-app --no-default-features --features local,formats,basic-checks,gold --locked
# Local editor only
cargo run -p studio-app --no-default-features --features local,formats,basic-checks --locked
```

Only the parity tooling and the linter parity tests need the private Babel data bundle (`tools/babel-data.json`); they skip when it is absent. See [development](docs/development.md) for checks, the contributor workflow and the extension SDK.

## Contributing

Bug reports, code, extensions, translations and accessibility fixes are welcome. Use a synthetic recording in reports and tests; never attach credentials, private tasks or someone else's audio. Discuss substantial changes in an [issue](https://github.com/Nominy/exaudio/issues) first.

## License

[MIT](LICENSE), including the author's first-party extensions. Dependencies, model weights and third-party extension packages keep their own licenses. Built on Rust, GPUI / gpui-kit and Wasmtime; the wordmark uses outlined Outfit lettering (SIL OFL 1.1). See [acknowledgements](docs/development.md#licenses-and-acknowledgements).
