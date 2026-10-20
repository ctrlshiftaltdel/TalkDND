#!/usr/bin/env bash
# Fail if a tracked file looks like a copyrighted book, a private save, or a DM secret.
# Prints matching paths only. Never prints file contents.
# Exit 0 when the index is clean, 1 when at least one path matches.
set -u

root=$(git rev-parse --show-toplevel) || exit 2
cd "$root" || exit 2

# 300 KiB. Tracked prose larger than this is treated as likely book text.
threshold=307200
fail=0

is_book_image() {
  case "$1" in
    *.png|*.PNG|*.jpg|*.JPG|*.jpeg|*.JPEG|*.gif|*.GIF|*.webp|*.WEBP|*.tif|*.TIF|*.tiff|*.TIFF|*.bmp|*.BMP)
      case "$2" in
        LetsPlay/Rules/*|LetsPlay/Adventures/*|LetsPlay/Saves/*|Rules/*|Adventures/*|Saves/*)
          return 0
          ;;
      esac
      ;;
  esac
  return 1
}

while IFS= read -r -d '' f; do
  base=${f##*/}
  hit=0

  case "$f" in
    *.pdf|*.PDF|*.epub|*.EPUB|*.docx|*.DOCX) hit=1 ;;
  esac

  if is_book_image "$base" "$f"; then
    hit=1
  fi

  case "$f" in
    LetsPlay/Rules/2014/README.md|LetsPlay/Rules/2024/README.md) ;;
    LetsPlay/Rules/2014/*|LetsPlay/Rules/2024/*) hit=1 ;;
    Rules/2014/README.md|Rules/2024/README.md) ;;
    Rules/2014/*|Rules/2024/*) hit=1 ;;
    LetsPlay/Adventures/README.md|Adventures/README.md) ;;
    LetsPlay/Adventures/*|Adventures/*) hit=1 ;;
    LetsPlay/Saves/README.md|Saves/README.md) ;;
    LetsPlay/Saves/*|Saves/*) hit=1 ;;
  esac

  case "$base" in
    dm-secrets.md|scarecrow.yaml) hit=1 ;;
  esac

  # Loose markdown directly under Rules/, plus any deeper *.md that is not a
  # README or a rulings file. Bash '*' matches slashes, so README.md is allowed
  # by basename and rulings*.md stays tracked.
  case "$f" in
    LetsPlay/Rules/*.md|Rules/*.md)
      case "$base" in
        README.md|rulings*.md) ;;
        *) hit=1 ;;
      esac
      ;;
  esac

  case "$f" in
    *.md|*.txt|*.MD|*.TXT)
      if [ -f "$f" ]; then
        size=$(wc -c < "$f" | tr -d ' ')
        if [ "$size" -gt "$threshold" ]; then
          hit=1
        fi
      fi
      ;;
  esac

  if [ "$hit" -eq 1 ]; then
    printf '%s\n' "$f"
    fail=1
  fi
done < <(git ls-files -z)

exit "$fail"
