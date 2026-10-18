#!/usr/bin/env bash
# Fast feedback loop for the app.
#
#   ./dev.sh harv [app args...]
#
# * Edit src/style.css  -> the running window restyles itself instantly (no rebuild, no restart).
# * Edit any .rs file   -> incremental debug build; on success the window is swapped for the
#                          new binary, on failure the errors are printed and the OLD window
#                          keeps running, so a typo never costs you your screen.
#
# App args are re-applied on every restart, so pass --play/--search/--select to land back in the
# state you were looking at. Needs `inotifywait` (inotify-tools).
set -u
pkg=${1:?usage: ./dev.sh <package> [app args...]}
shift
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"
bin="target/debug/$pkg"
dir=$(ls -d apps/"$pkg" crates/"$pkg" 2>/dev/null | head -1)
[ -n "$dir" ] || { echo "unknown package: $pkg" >&2; exit 1; }
pid=""

start() {
  "$bin" --watch "$@" &
  pid=$!
}
stop() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  pid=""
}
trap 'stop; exit 0' INT TERM

echo "── building $pkg (first build compiles dependencies optimised; later ones are incremental)"
if cargo build -p "$pkg" --color always; then start "$@"; else echo "── initial build failed; fix and save to retry"; fi

# CSS is excluded: the app reloads it itself. Editor temp files are ignored.
while inotifywait -qq -r -e close_write,moved_to \
    --exclude '(\.swp|\.swx|~|\.tmp|4913|\.css)$' \
    "$dir/src" crates/lyrics-core/src; do
  sleep 0.15 # let multi-file saves settle
  echo "── rebuilding…"
  if cargo build -p "$pkg" --color always; then
    stop
    start "$@"
    echo "── restarted"
  else
    echo "── build failed; previous window left running"
  fi
done
