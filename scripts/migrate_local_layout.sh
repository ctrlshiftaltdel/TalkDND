#!/bin/sh
# Move untracked local play content from the pre-LetsPlay layout into LetsPlay/.
# Dry-run by default. Pass --apply to move files.
# Idempotent: an existing destination is never overwritten.
set -eu

usage() {
  printf '%s\n' "usage: $0 [--apply]" >&2
  exit 2
}

apply=0
if [ "$#" -eq 0 ]; then
  apply=0
elif [ "$#" -eq 1 ] && [ "$1" = "--apply" ]; then
  apply=1
else
  usage
fi

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

moves=0
skips=0

log_move() {
  printf '%s\n' "move: $1 -> $2"
  moves=$((moves + 1))
}

log_skip() {
  printf '%s\n' "skip (exists): $1 -> $2"
  skips=$((skips + 1))
}

# Move src to dest. Directories merge. Files that already exist are left in place.
move_entry() {
  src=$1
  dest=$2
  if [ ! -e "$src" ] && [ ! -L "$src" ]; then
    return 0
  fi
  if [ -d "$src" ] && [ ! -L "$src" ] && [ -d "$dest" ] && [ ! -L "$dest" ]; then
    move_children "$src" "$dest"
    return 0
  fi
  if [ -e "$dest" ] || [ -L "$dest" ]; then
    log_skip "$src" "$dest"
    return 0
  fi
  log_move "$src" "$dest"
  if [ "$apply" -eq 1 ]; then
    mkdir -p "$(dirname -- "$dest")"
    mv -- "$src" "$dest"
  fi
}

move_children() {
  src_dir=$1
  dest_dir=$2
  if [ ! -d "$src_dir" ]; then
    return 0
  fi
  for child in "$src_dir"/* "$src_dir"/.[!.]* "$src_dir"/..?*; do
    if [ -e "$child" ] || [ -L "$child" ]; then
      name=$(basename -- "$child")
      move_entry "$child" "$dest_dir/$name"
    fi
  done
}

# Rules/2014 and Rules/2024: player books. README.md at the destination is tracked.
move_children "Rules/2014" "LetsPlay/Rules/2014"
move_children "Rules/2024" "LetsPlay/Rules/2024"

# Other loose files (and unexpected directories) directly under Rules/.
# Do not sweep Rules/combat; only its local overlay and scarecrow are private.
if [ -d Rules ]; then
  for child in Rules/* Rules/.[!.]* Rules/..?*; do
    if [ -e "$child" ] || [ -L "$child" ]; then
      name=$(basename -- "$child")
      case "$name" in
        2014|2024|combat) ;;
        *) move_entry "$child" "LetsPlay/Rules/$name" ;;
      esac
    fi
  done
fi

move_entry "Rules/combat/local" "LetsPlay/Rules/combat/local"
move_entry "Rules/combat/2024/monsters/scarecrow.yaml" "LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml"

move_children "Adventures" "LetsPlay/Adventures"
move_children "Saves" "LetsPlay/Saves"

if [ "$apply" -eq 1 ]; then
  rmdir Rules/combat/2024/monsters 2>/dev/null || true
  rmdir Rules/combat/2024 2>/dev/null || true
  rmdir Rules/combat 2>/dev/null || true
  rmdir Rules/2014 Rules/2024 Rules Adventures Saves 2>/dev/null || true
fi

if [ "$apply" -eq 1 ]; then
  printf '%s\n' "migrate_local_layout: applied, $moves move(s), $skips skip(s)"
else
  printf '%s\n' "migrate_local_layout: dry-run, $moves move(s), $skips skip(s) (pass --apply to move)"
fi
