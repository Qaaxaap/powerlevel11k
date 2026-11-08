#!/usr/bin/env bash
# Run a TUI in a headless sway compositor with a real kitty terminal, then capture the rendered
# pixels with grim. The captures are what the visual tests compare against their references:
# they contain the font raster, the terminal colours and the cursor state, which a byte stream
# cannot show.
#
# The environment is fixed so that the pixels are reproducible:
#   - no display server, no seat, one headless output;
#   - fixed XKB layout, locale and font configuration (the wrapper sets FONTCONFIG_FILE);
#   - gles2 with a DRM device, pixman without one;
#   - every wait is a condition poll, never a sleep guess: wayland socket, sway IPC, output
#     mode, kitty window, and a frame that stayed identical across two grim captures.
#
# Usage:
#   record-tui --out DIR [options] -- CMD [ARGS...]
#
# Options:
#   --out DIR             output directory (default ./record-tui-out)
#   --size WxH            output resolution (default 700x450)
#   --title NAME          kitty class/title and output file prefix (default tui)
#   --kitty-conf FILE     kitty.conf copied into the per-run XDG_CONFIG_HOME
#   --resize WxH          after the first frame, switch the output size and capture again;
#                         may be repeated, one frame per size
#   --type TEXT           after the window is ready, type TEXT and press Return; the capture
#                         then shows the prompt after that command (wtype sends the keys)
#   --type-wait SEC       wait after typing before the stable-frame capture (default 2)
#   --settle-before-type
#                         wait for a stable frame before typing, so the first keys do not
#                         arrive while the shell is still initializing
#   --wait-for-after-type FILE
#                         After Return, wait for this file. The command writes the file. Wait
#                         for it before --type-wait starts. Then the capture shows the finished
#                         command, not a frame in the middle of it.
#   --min-wait SEC        minimum wait after the window appears (default 1.5)
#   --settle-timeout SEC  upper bound for the stable-frame and window waits (default 15)
#   --wait-for FILE       wait for the TUI's ready marker instead of the minimum wait
#   --keep                keep the temporary directory and the sway log

set -euo pipefail

SIZE="700x450"
OUT="./record-tui-out"
TITLE="tui"
KITTY_CONF=""
TYPE_TEXT=""
TYPE_WAIT="2"
TYPE_WAIT_FOR=""
SETTLE_BEFORE_TYPE="0"
MIN_WAIT="1.5"
SETTLE_TIMEOUT="15"
WAIT_FOR=""
KEEP="0"
RESIZES=()

usage() {
  sed -n '2,36p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p' >&2
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    --title) TITLE="$2"; shift 2 ;;
    --kitty-conf) KITTY_CONF="$2"; shift 2 ;;
    --type) TYPE_TEXT="$2"; shift 2 ;;
    --type-wait) TYPE_WAIT="$2"; shift 2 ;;
    --wait-for-after-type) TYPE_WAIT_FOR="$2"; shift 2 ;;
    --settle-before-type) SETTLE_BEFORE_TYPE="1"; shift ;;
    --resize) RESIZES+=("$2"); shift 2 ;;
    --min-wait) MIN_WAIT="$2"; shift 2 ;;
    --settle-timeout) SETTLE_TIMEOUT="$2"; shift 2 ;;
    --wait-for) WAIT_FOR="$2"; shift 2 ;;
    --keep) KEEP="1"; shift ;;
    -h|--help) usage 0 ;;
    --) shift; break ;;
    *) echo "unknown argument: $1" >&2; usage 2 ;;
  esac
done
[ $# -gt 0 ] || { echo "missing the TUI command after --" >&2; usage 2; }

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
mkdir -p "$WORK/xdg" "$WORK/cache" "$WORK/config"
chmod 700 "$WORK/xdg"

SWAY_PID=""
DBUS_PID=""
cleanup() {
  if [ -n "$SWAY_PID" ]; then
    kill "$SWAY_PID" 2>/dev/null || true
  fi
  if [ -n "$DBUS_PID" ]; then
    kill "$DBUS_PID" 2>/dev/null || true
  fi
  wait 2>/dev/null || true
  if [ "$KEEP" = "1" ]; then
    echo "[record-tui] kept $WORK" >&2
  else
    rm -rf "$WORK" 2>/dev/null || true
  fi
}
trap cleanup EXIT

die() {
  echo "[record-tui] error: $*" >&2
  [ -f "$WORK/sway.log" ] && tail -20 "$WORK/sway.log" >&2
  exit 1
}

# ---- fixed environment ----
export XDG_RUNTIME_DIR="$WORK/xdg"
export XDG_CACHE_HOME="$WORK/cache"
export XDG_CONFIG_HOME="$WORK/config"
export WLR_BACKENDS=headless
export WLR_LIBINPUT_NO_DEVICES=1
export WLR_HEADLESS_OUTPUTS=1
export XKB_DEFAULT_LAYOUT=us
export XKB_DEFAULT_MODEL=pc105
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
# The reference frames are generated with the software renderer: pixman is deterministic and works
# without a GPU, so a local run and a CI run produce the same pixels. gles2 remains available by
# setting WLR_RENDERER explicitly.
export WLR_RENDERER="${WLR_RENDERER:-pixman}"

# The terminal has to render the font the reference frames were made with: the wrapper exports
# FONTCONFIG_FILE, and kitty.conf names the family from that configuration.
[ -n "${FONTCONFIG_FILE:-}" ] || die "FONTCONFIG_FILE is not set (run through the visual test wrapper)"
if [ -n "$KITTY_CONF" ]; then
  [ -f "$KITTY_CONF" ] || die "kitty.conf not found: $KITTY_CONF"
  mkdir -p "$XDG_CONFIG_HOME/kitty"
  cp "$KITTY_CONF" "$XDG_CONFIG_HOME/kitty/kitty.conf"
fi

CONF="$WORK/sway.conf"
cat > "$CONF" <<EOF
output HEADLESS-1 mode $SIZE
default_border none
focus_follows_mouse no
xwayland disable
EOF

# The Nix sway wrapper starts a private session bus when DBUS_SESSION_BUS_ADDRESS is unset, and
# that bus daemon reads /etc/dbus-1/session.conf, which does not exist in a cleared environment.
# Start the bus from the configuration the devShell exports and hand its address to sway.
dbus_daemon=${P11K_VISUAL_DBUS_DAEMON:?P11K_VISUAL_DBUS_DAEMON is not set}
dbus_conf=${P11K_VISUAL_DBUS_CONF:?P11K_VISUAL_DBUS_CONF is not set}
[ -f "$dbus_conf" ] || die "dbus session configuration not found: $dbus_conf"
dbus_info=$("$dbus_daemon" --fork --print-address=1 --print-pid=1 --config-file="$dbus_conf") ||
  die "dbus-daemon failed to start"
DBUS_SESSION_BUS_ADDRESS=$(printf '%s\n' "$dbus_info" | sed -n 1p)
DBUS_PID=$(printf '%s\n' "$dbus_info" | sed -n 2p)
[ -n "$DBUS_SESSION_BUS_ADDRESS" ] || die "dbus-daemon did not print a bus address"
[ -n "$DBUS_PID" ] || die "dbus-daemon did not print a pid"
export DBUS_SESSION_BUS_ADDRESS

sway -c "$CONF" > "$WORK/sway.log" 2>&1 &
SWAY_PID=$!

# Wait for the wayland socket.
WAYLAND_DISPLAY=""
for _ in $(seq 1 3000); do
  for f in "$XDG_RUNTIME_DIR"/wayland-*; do
    case "$f" in *.lock) continue ;; esac
    [ -e "$f" ] || continue
    WAYLAND_DISPLAY=${f##*/}
    break
  done
  [ -n "$WAYLAND_DISPLAY" ] && break
  sleep 0.01
done
[ -n "$WAYLAND_DISPLAY" ] || die "sway did not create a wayland socket"
export WAYLAND_DISPLAY

# Wait for the sway IPC socket.
SWAYSOCK=""
for _ in $(seq 1 3000); do
  for f in "$XDG_RUNTIME_DIR"/sway-ipc.*.sock; do
    [ -e "$f" ] || continue
    SWAYSOCK=$f
    break
  done
  [ -n "$SWAYSOCK" ] && break
  sleep 0.01
done
[ -n "$SWAYSOCK" ] || die "sway did not create an IPC socket"
export SWAYSOCK

OUTPUT=""
for _ in $(seq 1 2000); do
  OUTPUT=$(swaymsg -t get_outputs 2>/dev/null | jq -r '.[0].name // empty' 2>/dev/null || true)
  [ -n "$OUTPUT" ] && break
  sleep 0.01
done
[ -n "$OUTPUT" ] || die "sway has no output"

wait_size() {
  local want=$1 w h
  w=${want%x*}
  h=${want#*x}
  for _ in $(seq 1 2000); do
    if swaymsg -t get_outputs 2>/dev/null | jq -e --argjson w "$w" --argjson h "$h" \
      '.[0].current_mode | .width == $w and .height == $h' >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.01
  done
  return 1
}

if ! wait_size "$SIZE"; then
  swaymsg output "$OUTPUT" mode "$SIZE" >/dev/null 2>&1 || true
  wait_size "$SIZE" || die "output did not switch to $SIZE"
fi

# ---- start kitty and the TUI ----
KITTY_SOCKET="$XDG_RUNTIME_DIR/kitty.sock"
CMD=$(printf '%q ' "$@")
swaymsg exec "kitty --class $TITLE --title $TITLE -o allow_remote_control=yes --listen-on unix:$KITTY_SOCKET --hold $CMD" >/dev/null 2>&1

for _ in $(seq 1 6000); do
  if swaymsg -t get_tree -r 2>/dev/null | grep -q "\"app_id\": \"$TITLE\""; then
    break
  fi
  sleep 0.01
done
swaymsg -t get_tree -r 2>/dev/null | grep -q "\"app_id\": \"$TITLE\"" ||
  die "kitty window did not appear"

if [ -n "$WAIT_FOR" ]; then
  for _ in $(seq 1 $((SETTLE_TIMEOUT * 100))); do
    [ -e "$WAIT_FOR" ] && break
    sleep 0.01
  done
  [ -e "$WAIT_FOR" ] || die "ready marker did not appear: $WAIT_FOR"
else
  sleep "$MIN_WAIT"
fi

# Settle: three identical grim captures in a row, with a 50 ms gap. The cursor blink is
# disabled in kitty.conf. Without that setting, a frame is rarely stable.
settle() {
  local stable=0
  local prev=""
  for _ in $(seq 1 $((SETTLE_TIMEOUT * 20))); do
    grim -o "$OUTPUT" "$WORK/settle.png" 2>/dev/null || true
    if [ -s "$WORK/settle.png" ]; then
      local cur
      cur=$(sha256sum "$WORK/settle.png" | cut -d' ' -f1)
      if [ "$cur" = "$prev" ]; then
        stable=$((stable + 1))
        [ "$stable" -ge 3 ] && return 0
      else
        stable=0
        prev=$cur
      fi
    fi
    sleep 0.05
  done
  echo "[record-tui] warning: no stable frame within ${SETTLE_TIMEOUT}s" >&2
  return 0
}

if [ -n "$TYPE_TEXT" ] && [ "$SETTLE_BEFORE_TYPE" = "1" ]; then
  # The shell may still draw its first prompt. A stable frame here means the first keys cannot
  # arrive while the shell initializes.
  settle
fi

if [ -n "$TYPE_TEXT" ]; then
  # Type through kitty's remote-control socket instead of the compositor's virtual keyboard:
  # wtype loses the first key event while the window gains focus, which silently changed
  # `sleep 1` into `leep 1` and made the shell report a missing command.
  for _ in $(seq 1 $((SETTLE_TIMEOUT * 100))); do
    [ -S "$KITTY_SOCKET" ] && break
    sleep 0.01
  done
  [ -S "$KITTY_SOCKET" ] || die "kitty did not create its remote-control socket: $KITTY_SOCKET"
  kitten @ --to "unix:$KITTY_SOCKET" send-text "$TYPE_TEXT"
  # Let the line editor consume the text before Return. A fixed sleep is too short on a slow
  # CI runner: the line editor drops Return while it still processes the text.
  settle
  sleep 0.05
  kitten @ --to "unix:$KITTY_SOCKET" send-key Return
  if [ -n "$TYPE_WAIT_FOR" ]; then
    # The command writes this marker when it finishes. Wait for the marker before the fixed
    # TYPE_WAIT. The capture then occurs after the command completed.
    for _ in $(seq 1 $((SETTLE_TIMEOUT * 100))); do
      [ -e "$TYPE_WAIT_FOR" ] && break
      sleep 0.01
    done
    if [ ! -e "$TYPE_WAIT_FOR" ]; then
      # Keep the frame for diagnosis: the caller uploads the capture directory when a run fails.
      grim -o "$OUTPUT" "$OUT/command-timeout.png" 2>/dev/null || true
      die "command marker did not appear: $TYPE_WAIT_FOR"
    fi
  fi
  sleep "$TYPE_WAIT"
fi


# ---- capture ----
n=0
snap() {
  local f
  f=$(printf '%s/%s-%03d.png' "$OUT" "$TITLE" "$n")
  grim -o "$OUTPUT" "$f"
  echo "[record-tui] $f"
  n=$((n + 1))
}

snap
for r in "${RESIZES[@]}"; do
  swaymsg output "$OUTPUT" mode "$r" >/dev/null 2>&1 || die "cannot switch the output to $r"
  wait_size "$r" || die "output did not switch to $r"
  sleep 0.05
  snap
done

echo "[record-tui] renderer=$WLR_RENDERER output=$OUTPUT size=$SIZE" >&2
