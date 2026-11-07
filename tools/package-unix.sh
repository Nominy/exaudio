#!/usr/bin/env bash
# Builds Exaudio natively on macOS or Linux and packs it with Velopack.
#
# UPDATE_CHANNEL=stable packs the per-platform stable channel that one GitHub release carries
# for every platform (Velopack channel = runtime: osx-arm64, osx-x64, linux-x64, linux-arm64).
# On macOS the stable asset is a drag-to-Applications disk image (Exaudio-mac-<arch>.dmg)
# built from the Velopack-packed Exaudio.app; on Linux it is the Velopack AppImage
# (Exaudio-linux-<arch>.AppImage). UPDATE_CHANNEL=alpha packs channel `alpha` for the VPS
# alpha feed (macOS: the portable zip, as macos-alpha.yml publishes it).
#
# Env: UPDATE_CHANNEL (stable | alpha, required), VERSION (default: crates/studio-app version;
# baked into the binary as STUDIO_BUILD_VERSION), RUNTIME (osx-arm64 | osx-x64 | linux-x64 |
# linux-arm64, default: host), RELEASE_NOTES (markdown file embedded by vpk), STAGE_DIR,
# OUTPUT_DIR, PUBLISH_DIR, STUDIO_ALPHA_FEED_URL (alpha feed the build updates from, default:
# the platform feed), MACOSX_DEPLOYMENT_TARGET (default 14.0), optional APPLE_* signing
# inputs below. Outputs: the full local feed in OUTPUT_DIR (default target/velopack/Releases)
# and, for publication, the current release's assets + feed metadata + publish-manifest.json
# in PUBLISH_DIR (default target/velopack/Publish).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

update_channel="${UPDATE_CHANNEL:-}"
version="${VERSION:-}"
pack_id="Exaudio"
pack_title="Exaudio"
pack_authors="Nominy"
main_exe="Exaudio"
bundle_id="io.github.nominy.exaudio"
stage_dir="${STAGE_DIR:-$repo_root/target/velopack/unix-stage}"
output_dir="${OUTPUT_DIR:-$repo_root/target/velopack/Releases}"
publish_dir="${PUBLISH_DIR:-$repo_root/target/velopack/Publish}"
release_notes="${RELEASE_NOTES:-}"
# Cargo's own CARGO_TARGET_DIR, when set, decides where the release binaries land.
source_dir="${CARGO_TARGET_DIR:-$repo_root/target}/release"
runtime="${RUNTIME:-}"
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-14.0}"

case "$(uname -s)" in
  Darwin) host_os="osx" ;;
  Linux) host_os="linux" ;;
  *)
    echo "package-unix.sh runs on macOS or Linux; use tools/package-windows.ps1 on Windows." >&2
    exit 1
    ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) host_arch="arm64" ;;
  x86_64) host_arch="x64" ;;
  *)
    echo "Could not infer the Velopack runtime from uname -m; set RUNTIME explicitly." >&2
    exit 1
    ;;
esac
host_runtime="$host_os-$host_arch"

case "$update_channel" in
  stable | alpha) ;;
  *)
    echo "UPDATE_CHANNEL must be 'stable' or 'alpha' (got '${update_channel}')." >&2
    exit 1
    ;;
esac

if [[ -z "$version" ]]; then
  version="$(awk -F '"' '/^version[[:space:]]*=/ { print $2; exit }' "$repo_root/crates/studio-app/Cargo.toml")"
fi
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "VERSION must be SemVer (major.minor.patch[-prerelease]): '$version'" >&2
  exit 1
fi

if [[ -z "$runtime" ]]; then
  runtime="$host_runtime"
fi
if [[ "$runtime" != "$host_runtime" ]]; then
  echo "Requested RUNTIME=$runtime, but this host builds $host_runtime. The script does not cross-compile." >&2
  exit 1
fi
# Stable: one Velopack channel per OS/CPU so one GitHub release serves every platform.
if [[ "$update_channel" == "stable" ]]; then
  channel="$runtime"
else
  channel="alpha"
fi
case "$runtime" in
  osx-arm64) user_asset="$pack_id-mac-arm64.dmg" ;;
  osx-x64) user_asset="$pack_id-mac-x64.dmg" ;;
  linux-*) user_asset="$pack_id-$channel.AppImage" ;;
esac

if [[ -n "$release_notes" && ! -f "$release_notes" ]]; then
  echo "RELEASE_NOTES file does not exist: $release_notes" >&2
  exit 1
fi
if ! command -v dotnet >/dev/null 2>&1; then
  echo "The repo-local vpk tool requires .NET SDK 8. Run: dotnet tool restore" >&2
  exit 1
fi

run_vpk() {
  (cd "$repo_root" && dotnet tool run vpk -- "$@")
}

python3 - "$repo_root" "$stage_dir" "$output_dir" "$publish_dir" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
names = ["STAGE_DIR", "OUTPUT_DIR", "PUBLISH_DIR"]
paths = [pathlib.Path(value).resolve() for value in sys.argv[2:]]
for path in paths:
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise SystemExit(f"refusing to operate outside repo root: {path}") from error
    if not relative.parts:
        raise SystemExit(f"refusing to use repo root as a release directory: {path}")
for left_index, left in enumerate(paths):
    for right_index in range(left_index + 1, len(paths)):
        right = paths[right_index]
        if left == right or left in right.parents or right in left.parents:
            raise SystemExit(
                f"{names[left_index]} and {names[right_index]} must not overlap: "
                f"{left} and {right}"
            )
PY

# Baked into the binary: the version it reports and the update channel it follows.
export STUDIO_BUILD_VERSION="$version"
export STUDIO_UPDATE_CHANNEL="$update_channel"

echo "Packaging Exaudio $version for $runtime ($update_channel, Velopack channel $channel)."
if [[ "$host_os" == "osx" ]]; then
  export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
  echo "MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
fi
if [[ "$update_channel" == "alpha" && -n "${STUDIO_ALPHA_FEED_URL:-}" ]]; then
  echo "STUDIO_ALPHA_FEED_URL=$STUDIO_ALPHA_FEED_URL"
fi

# Build the app, relay and isolated compiler together so shared dependency features stay unified.
(cd "$repo_root" && cargo build -p studio-app -p studio-relay -p exaudio-wasm --release --locked)

for binary in studio-app studio-relay exaudio-wasm-worker; do
  if [[ ! -f "$source_dir/$binary" ]]; then
    echo "Missing release executable: $source_dir/$binary" >&2
    exit 1
  fi
done

rm -rf "$stage_dir"
mkdir -p "$stage_dir" "$output_dir"
cp "$source_dir/studio-app" "$stage_dir/$main_exe"
cp "$source_dir/studio-relay" "$stage_dir/studio-relay"
# Isolated compiler child for untrusted extension .wasm; the app resolves it as its sibling.
cp "$source_dir/exaudio-wasm-worker" "$stage_dir/exaudio-wasm-worker"
chmod +x "$stage_dir/$main_exe" "$stage_dir/studio-relay" "$stage_dir/exaudio-wasm-worker"
mkdir -p "$stage_dir/assets/demo"
cp "$repo_root/crates/studio-editor/assets/demo/speaker_1.wav" "$stage_dir/assets/demo/"
cp "$repo_root/crates/studio-editor/assets/demo/speaker_2.wav" "$stage_dir/assets/demo/"

for file in README.md LICENSE; do
  if [[ -f "$repo_root/$file" ]]; then
    cp "$repo_root/$file" "$stage_dir/"
  fi
done

pack_args=(
  --packId "$pack_id"
  --packVersion "$version"
  --packDir "$stage_dir"
  --mainExe "$main_exe"
  --channel "$channel"
  --runtime "$runtime"
  --packTitle "$pack_title"
  --packAuthors "$pack_authors"
  --outputDir "$output_dir"
)
if [[ -n "$release_notes" ]]; then
  pack_args+=(--releaseNotes "$(cd "$(dirname "$release_notes")" && pwd -P)/$(basename "$release_notes")")
fi

if [[ "$host_os" == "osx" ]]; then
  # The bundle icon: an .icns rendered from the 512 px app icon (vpk accepts .icns only).
  iconset="$stage_dir-icon/Exaudio.iconset"
  icns="$stage_dir-icon/Exaudio.icns"
  rm -rf "$stage_dir-icon"
  mkdir -p "$iconset"
  for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$repo_root/assets/app/exaudio-512.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size * 2))" "$((size * 2))" "$repo_root/assets/app/exaudio-512.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$icns"
  pack_args+=(--icon "$icns" --bundleId "$bundle_id")
  # Optional signing inputs:
  # APPLE_DEVELOPER_ID_APPLICATION="Developer ID Application: Example, Inc. (TEAMID)"
  # APPLE_DEVELOPER_ID_INSTALLER="Developer ID Installer: Example, Inc. (TEAMID)"
  # APPLE_NOTARY_PROFILE="notarytool-keychain-profile"
  if [[ -n "${APPLE_DEVELOPER_ID_APPLICATION:-}" ]]; then
    pack_args+=(--signAppIdentity "$APPLE_DEVELOPER_ID_APPLICATION")
  fi
  if [[ -n "${APPLE_DEVELOPER_ID_INSTALLER:-}" ]]; then
    pack_args+=(--signInstallIdentity "$APPLE_DEVELOPER_ID_INSTALLER")
  else
    pack_args+=(--noInst)
  fi
  if [[ -n "${APPLE_CODESIGN_ENTITLEMENTS:-}" ]]; then
    pack_args+=(--signEntitlements "$APPLE_CODESIGN_ENTITLEMENTS")
  fi
  if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then
    pack_args+=(--notaryProfile "$APPLE_NOTARY_PROFILE")
  fi
  if [[ -n "${APPLE_KEYCHAIN:-}" ]]; then
    pack_args+=(--keychain "$APPLE_KEYCHAIN")
  fi
else
  # The AppImage's desktop entry icon.
  pack_args+=(--icon "$repo_root/assets/app/exaudio-512.png")
fi

run_vpk pack "${pack_args[@]}"

# Stable macOS: the user-facing asset is a disk image of the Velopack-packed app (its
# Contents/MacOS/sq.version and UpdateMac keep in-app updates working). Portable.zip stays in
# the local feed only; the alpha VPS feed keeps publishing the zip itself.
if [[ "$host_os" == "osx" && "$update_channel" == "stable" ]]; then
  portable_zip="$output_dir/$pack_id-$channel-Portable.zip"
  if [[ ! -f "$portable_zip" ]]; then
    echo "vpk pack did not produce $portable_zip" >&2
    exit 1
  fi
  dmg_root="$stage_dir-dmg"
  rm -rf "$dmg_root"
  mkdir -p "$dmg_root"
  ditto -x -k "$portable_zip" "$dmg_root"
  if [[ ! -d "$dmg_root/$pack_id.app" ]]; then
    echo "$portable_zip does not contain $pack_id.app" >&2
    exit 1
  fi
  # Without a Developer ID the bundle leaves vpk unsigned, and macOS reports an unsigned
  # download as "damaged" with no way to allow it. An ad-hoc signature turns that into the
  # ordinary unidentified-developer prompt, which System Settings can override.
  if [[ -z "${APPLE_DEVELOPER_ID_APPLICATION:-}" ]]; then
    codesign --force --deep --sign - "$dmg_root/$pack_id.app"
    codesign --verify --deep --strict "$dmg_root/$pack_id.app"
    rm -f "$portable_zip"
    ditto -c -k --keepParent "$dmg_root/$pack_id.app" "$portable_zip"
  fi
  ln -s /Applications "$dmg_root/Applications"
  rm -f "$output_dir/$user_asset"
  hdiutil create -quiet -volname "$pack_title" -srcfolder "$dmg_root" -fs HFS+ -format UDZO "$output_dir/$user_asset"
  hdiutil verify -quiet "$output_dir/$user_asset"
  rm -rf "$dmg_root"
fi

if [[ "$update_channel" == "stable" && ! -f "$output_dir/$user_asset" ]]; then
  echo "vpk pack did not produce $user_asset" >&2
  exit 1
fi

python3 - "$output_dir" "$publish_dir" "$channel" "$version" "$update_channel" "$host_os" "$user_asset" <<'PY'
import hashlib
import json
import pathlib
import shutil
import sys

output = pathlib.Path(sys.argv[1]).resolve()
publish = pathlib.Path(sys.argv[2]).resolve()
channel = sys.argv[3]
version = sys.argv[4]
update_channel = sys.argv[5]
host_os = sys.argv[6]
user_asset = sys.argv[7]
asset_index_name = f"assets.{channel}.json"
asset_index_path = output / asset_index_name
if not asset_index_path.is_file():
    raise SystemExit(f"missing Velopack asset index: {asset_index_path}")

with asset_index_path.open("r", encoding="utf-8-sig") as handle:
    assets = json.load(handle)
if not isinstance(assets, list) or not assets:
    raise SystemExit(f"Velopack asset index contains no current assets: {asset_index_path}")

files = []
seen = set()

def add(relative_name, kind):
    if not isinstance(relative_name, str) or not relative_name:
        raise SystemExit("publish manifest contains an empty relative path")
    pure = pathlib.PurePosixPath(relative_name)
    if (
        pure.is_absolute()
        or any(part in ("", ".", "..") for part in pure.parts)
        or "\\" in relative_name
        or "\n" in relative_name
        or "\r" in relative_name
    ):
        raise SystemExit(f"unsafe publish path: {relative_name!r}")
    source = (output / pathlib.Path(*pure.parts)).resolve()
    try:
        source.relative_to(output)
    except ValueError as error:
        raise SystemExit(f"publish path escapes output directory: {relative_name}") from error
    folded = relative_name.casefold()
    if folded in seen:
        raise SystemExit(f"duplicate publish path: {relative_name}")
    if not source.is_file():
        raise SystemExit(f"publish manifest references a missing file: {relative_name}")
    seen.add(folded)
    files.append((relative_name, kind, source))

# Stable macOS publishes the disk image instead of Velopack's portable zip.
skip_portable = host_os == "osx" and update_channel == "stable"
for asset in assets:
    if not isinstance(asset, dict):
        raise SystemExit("Velopack asset index entry is not an object")
    name = asset.get("RelativeFileName")
    if skip_portable and isinstance(name, str) and name.endswith("-Portable.zip"):
        continue
    add(name, "asset")
if update_channel == "stable" and user_asset.casefold() not in seen:
    add(user_asset, "asset")
for metadata_name in (
    asset_index_name,
    f"RELEASES-{channel}",
    f"releases.{channel}.json",
):
    add(metadata_name, "metadata")

if publish.exists():
    shutil.rmtree(publish)
publish.mkdir(parents=True)
manifest_files = []
for relative_name, kind, source in files:
    destination = publish / pathlib.Path(*pathlib.PurePosixPath(relative_name).parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    manifest_files.append(
        {
            "RelativeFileName": relative_name,
            "Kind": kind,
            "Size": destination.stat().st_size,
            "SHA256": hashlib.sha256(destination.read_bytes()).hexdigest().upper(),
        }
    )

manifest = {"Version": version, "Channel": channel, "Files": manifest_files}
manifest_path = publish / "publish-manifest.json"
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
publish_bytes = sum(path.stat().st_size for path in publish.rglob("*") if path.is_file())
print(
    f"PublishDir contains {len(manifest_files)} indexed files plus its manifest "
    f"({publish_bytes} bytes): {publish}"
)
PY

echo "Velopack $update_channel artifacts (channel $channel):"
find "$publish_dir" -maxdepth 1 -type f -print | sort
