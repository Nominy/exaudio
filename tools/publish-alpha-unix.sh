#!/usr/bin/env bash
# Publishes a Velopack publish payload (tools/package-unix.sh PUBLISH_DIR) to a per-target VPS
# alpha feed over SSH: upload to an unserved incoming directory, verify every file against
# publish-manifest.json, refuse to replace an existing versioned package with different bytes,
# promote assets first and the feed metadata last, keep the previous metadata for rollback.
#
# Env: PUBLISH_DIR (required), REMOTE_PATH (live feed directory, e.g.
# /var/www/exaudio/alpha/linux-x64), FEED_SUFFIX (incoming subdirectory, e.g. linux-x64),
# CHANNEL (default alpha), ALPHA_SSH_HOST, ALPHA_SSH_USER, ALPHA_SSH_PRIVATE_KEY,
# ALPHA_SSH_KNOWN_HOSTS (required), ALPHA_SSH_PORT (default 22).
# Remote paths are validated here and expanded client-side on purpose.
# shellcheck disable=SC2029
set -euo pipefail

: "${PUBLISH_DIR:?PUBLISH_DIR is required.}"
: "${REMOTE_PATH:?REMOTE_PATH is required.}"
: "${FEED_SUFFIX:?FEED_SUFFIX is required.}"
: "${ALPHA_SSH_HOST:?ALPHA_SSH_HOST secret is required to publish.}"
: "${ALPHA_SSH_USER:?ALPHA_SSH_USER secret is required to publish.}"
: "${ALPHA_SSH_PRIVATE_KEY:?ALPHA_SSH_PRIVATE_KEY secret is required to publish.}"
: "${ALPHA_SSH_KNOWN_HOSTS:?ALPHA_SSH_KNOWN_HOSTS secret is required to publish.}"
channel="${CHANNEL:-alpha}"
if [[ ! "$REMOTE_PATH" =~ ^/[A-Za-z0-9._/-]+$ ]] || \
   [[ "$REMOTE_PATH" == *"//"* ]] || \
   [[ "$REMOTE_PATH" =~ (^|/)\.\.(/|$) ]]; then
  echo "Unsafe remote path: $REMOTE_PATH" >&2
  exit 1
fi
if [[ ! "$FEED_SUFFIX" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "Unsafe feed suffix: $FEED_SUFFIX" >&2
  exit 1
fi
remote_platform_root="/var/tmp/exaudio-alpha-incoming/$FEED_SUFFIX"
if [[ "$REMOTE_PATH" == "$remote_platform_root" ]] || \
   [[ "$REMOTE_PATH" == "$remote_platform_root/"* ]] || \
   [[ "$remote_platform_root" == "$REMOTE_PATH/"* ]]; then
  echo "Remote live and incoming paths must not overlap: $REMOTE_PATH and $remote_platform_root" >&2
  exit 1
fi

port="${ALPHA_SSH_PORT:-22}"
secrets_dir="$(mktemp -d)"
trap 'rm -rf "$secrets_dir"' EXIT
key_path="$secrets_dir/alpha_deploy_key"
known_hosts="$secrets_dir/alpha_known_hosts"
printf '%s\n' "$ALPHA_SSH_PRIVATE_KEY" > "$key_path"
printf '%s\n' "$ALPHA_SSH_KNOWN_HOSTS" > "$known_hosts"
chmod 600 "$key_path" "$known_hosts"

manifest="$PUBLISH_DIR/publish-manifest.json"
version="$(python3 - "$manifest" <<'PY'
import json
import sys
with open(sys.argv[1], "r", encoding="utf-8-sig") as handle:
    print(json.load(handle)["Version"])
PY
)"
if [[ ! "$version" =~ ^[0-9A-Za-z][0-9A-Za-z.+-]*$ ]]; then
  echo "Unsafe publish version: $version" >&2
  exit 1
fi
manifest_hash="$(shasum -a 256 "$manifest" | cut -d ' ' -f 1)"
deployment_id="$version-${manifest_hash:0:16}"
remote_deployment="$remote_platform_root/deployments/$deployment_id"
remote_payload="$remote_deployment/Publish"
remote_rollback_root="$remote_platform_root/rollbacks"
remote_lock="$remote_platform_root/publish.lock"

target="${ALPHA_SSH_USER}@${ALPHA_SSH_HOST}"
host_key_args=(-o StrictHostKeyChecking=yes -o UserKnownHostsFile="$known_hosts")
ssh_args=(-i "$key_path" -p "$port" "${host_key_args[@]}")
scp_args=(-i "$key_path" -P "$port" "${host_key_args[@]}")

ssh "${ssh_args[@]}" "$target" \
  "rm -rf '$remote_deployment' && mkdir -p '$remote_deployment'"
scp "${scp_args[@]}" -r "$PUBLISH_DIR" "${target}:$remote_payload"
ssh "${ssh_args[@]}" "$target" sh -s -- \
  "$remote_payload" "$REMOTE_PATH" "$remote_rollback_root" \
  "$remote_lock" "$channel" "$deployment_id" <<'REMOTE'
set -eu
incoming=$1
live=$2
rollback_root=$3
lock_path=$4
channel=$5
deployment_id=$6
manifest="$incoming/publish-manifest.json"

verify_manifest() {
  root=$1
  kind=${2:-all}
  python3 - "$root" "$manifest" "$kind" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
kind = sys.argv[3]
with open(sys.argv[2], "r", encoding="utf-8-sig") as handle:
    manifest = json.load(handle)
for entry in manifest["Files"]:
    if kind != "all" and entry["Kind"] != kind:
        continue
    relative = entry["RelativeFileName"]
    pure = pathlib.PurePosixPath(relative)
    if (
        not relative
        or pure.is_absolute()
        or any(part in ("", ".", "..") for part in pure.parts)
        or "\\" in relative
        or "\n" in relative
        or "\r" in relative
    ):
        raise SystemExit(f"unsafe publish path: {relative!r}")
    path = (root / pathlib.Path(*pure.parts)).resolve()
    try:
        path.relative_to(root)
    except ValueError as error:
        raise SystemExit(f"publish path escapes root: {relative}") from error
    if not path.is_file():
        raise SystemExit(f"missing publish file: {relative}")
    if path.stat().st_size != int(entry["Size"]):
        raise SystemExit(f"publish size mismatch: {relative}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest.casefold() != str(entry["SHA256"]).casefold():
        raise SystemExit(f"publish SHA-256 mismatch: {relative}")
PY
}

list_assets() {
  python3 - "$manifest" <<'PY'
import json
import sys
with open(sys.argv[1], "r", encoding="utf-8-sig") as handle:
    manifest = json.load(handle)
for entry in manifest["Files"]:
    if entry["Kind"] == "asset":
        print(entry["RelativeFileName"])
PY
}

promote_file() {
  relative=$1
  source="$incoming/$relative"
  destination="$live/$relative"
  mkdir -p "$(dirname "$destination")"
  if [ -f "$destination" ] && cmp -s "$source" "$destination"; then
    return
  fi
  temporary="$destination.incoming-$deployment_id"
  cp -p "$source" "$temporary"
  mv -f "$temporary" "$destination"
}

verify_manifest "$incoming" all
mkdir -p "$live" "$rollback_root" "$(dirname "$lock_path")"
exec 9>"$lock_path"
flock 9

# Versioned packages are immutable: old feed hashes and rollback metadata depend on them.
list_assets | while IFS= read -r relative; do
  case "$relative" in
    *.nupkg)
      if [ -f "$live/$relative" ] && ! cmp -s "$incoming/$relative" "$live/$relative"; then
        printf 'Refusing to replace existing versioned package with different bytes: %s\n' \
          "$relative" >&2
        exit 1
      fi
      ;;
  esac
done

rollback="$rollback_root/$(date -u +%Y%m%dT%H%M%SZ)-$deployment_id"
mkdir -p "$rollback"
for relative in "assets.$channel.json" "RELEASES-$channel" "releases.$channel.json"; do
  if [ -f "$live/$relative" ]; then
    cp -p "$live/$relative" "$rollback/$relative"
  fi
done

list_assets | while IFS= read -r relative; do
  promote_file "$relative"
done
verify_manifest "$live" asset

# Promote the primary update feed last, after every referenced package exists.
promote_file "assets.$channel.json"
promote_file "RELEASES-$channel"
promote_file "releases.$channel.json"
verify_manifest "$live" metadata
rm -rf "$incoming"
printf 'Published %s to %s; previous metadata: %s\n' \
  "$deployment_id" "$live" "$rollback"
REMOTE
ssh "${ssh_args[@]}" "$target" \
  "find '$REMOTE_PATH' -maxdepth 1 -type f -printf '%f %s bytes\n' | sort"
