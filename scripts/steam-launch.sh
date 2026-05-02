#!/usr/bin/env bash
set -u

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <id>" >&2
  exit 2
fi

id="$1"
candidates=("$id")

if [[ "$id" =~ ^[0-9]+$ ]]; then
  if (( id > 2147483647 )); then
    signed=$((id - 4294967296))
    candidates+=("$signed")
  fi
fi

try_cmd() {
  "$@" >/dev/null 2>&1
}

for candidate in "${candidates[@]}"; do
  uri_rungameid="steam://rungameid/$candidate"
  uri_launch="steam://launch/$candidate"

  try_cmd xdg-open "$uri_rungameid" && exit 0
  try_cmd xdg-open "$uri_launch" && exit 0

  try_cmd gio open "$uri_rungameid" && exit 0
  try_cmd gio open "$uri_launch" && exit 0

  try_cmd steam "$uri_rungameid" && exit 0
  try_cmd steam "$uri_launch" && exit 0

  try_cmd steam -applaunch -- "$candidate" && exit 0

  try_cmd flatpak run com.valvesoftware.Steam "$uri_rungameid" && exit 0
  try_cmd flatpak run com.valvesoftware.Steam "$uri_launch" && exit 0
done

exit 1
