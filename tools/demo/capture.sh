#!/bin/sh
# Regenerate the images in docs/images/.
#
# The engine draws the prompt itself, so the recording never has to emulate a
# terminal: `record.py pty` runs it in a pty of a fixed size and keeps the bytes,
# and the still images come from tmux pane dumps, which is also what makes the
# bash and fish panes work (both ask the terminal questions at startup).
#
# Needs: a built p11k (cargo build --release -p p11k-engine), zsh, bash, fish,
# python3, tmux, agg (asciinema/agg) and ImageMagick. cargo is only needed to
# record the execution-time part of the demo.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
images=$root/docs/images

mkdir -p "$images"

P11K=${P11K:-$root/target/release/p11k}
AGG=${AGG:-agg}
TMUX=${TMUX:-tmux}
MAGICK=${MAGICK:-magick}
FONT=${FONT:-"JetBrainsMono Nerd Font Mono"}
# ImageMagick wants a font file, agg wants the family name.
if [ -z "${FONT_FILE:-}" ] && command -v fc-match >/dev/null; then
    FONT_FILE=$(fc-match -f "%{file}" "$FONT")
fi
COLS=${COLS:-94}
LABEL_WIDTH=${LABEL_WIDTH:-1350}

export DEMO_HOME=${DEMO_HOME:-$here/home}
export DEMO_THEME=${DEMO_THEME:-$here/demo.kdl}
export P11K

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ ! -x "$P11K" ]; then
    echo "no p11k at $P11K — build it with: cargo build --release -p p11k-engine" >&2
    exit 1
fi

echo "== demo environment =="
HOME_DIR=$DEMO_HOME sh "$here/setup-demo.sh"

echo "== recording the animated demo =="
python3 "$here/record.py" pty "$here/scenes/demo.json" -o "$work/demo.cast"
"$AGG" -q --font-family "$FONT" --font-size 20 --theme asciinema \
    --idle-time-limit 1.0 --fps-cap 20 --speed 1.2 "$work/demo.cast" "$images/demo.gif"

# One pane per shell or preset, dumped from tmux and turned into a still.
# $1 shell, $2 theme arguments (--config PATH or --preset NAME), $3 label
pane() {
    shell=$1
    args=$2
    label=$3
    stem=$work/pane-$label

    case $shell in
        fish) rc=$DEMO_HOME/demo.fish ;;
        *) rc=$DEMO_HOME/.zshrc ;;
    esac

    "$TMUX" -L p11k-images kill-server 2>/dev/null || true
    "$TMUX" -L p11k-images -f /dev/null new-session -d -x "$COLS" -y 6 -c "$DEMO_HOME" \
        "env HOME=$DEMO_HOME P11K_USER_ZSHRC=$rc PATH=$DEMO_HOME/.cargo/bin:/usr/bin:/bin \
         $P11K --shell $shell --config $args"

    # Wait for the prompt to be on screen, then step into the demo repository.
    i=0
    while [ $i -lt 60 ]; do
        "$TMUX" -L p11k-images capture-pane -p -t 0 | grep -q '❯' && break
        i=$((i + 1))
        sleep 0.1
    done
    "$TMUX" -L p11k-images send-keys -t 0 'cd dev/p11k' Enter
    sleep 1.5

    # -S -2: the last two lines are exactly the current prompt (header + input).
    "$TMUX" -L p11k-images capture-pane -e -p -t 0 -S -2 > "$stem.ans"
    "$TMUX" -L p11k-images kill-server 2>/dev/null || true

    python3 "$here/record.py" screen "$stem.ans" -o "$stem.cast" \
        --cols "$COLS" --rows 2
    "$AGG" -q --font-family "$FONT" --font-size 20 --theme asciinema --select 100% \
        "$stem.cast" "$stem.png"
    "$MAGICK" "$stem.png" -background '#121314' -gravity east \
        -extent "$LABEL_WIDTH"x \
        -fill '#8f8f8f' -font "$FONT_FILE" -pointsize 18 -gravity west -annotate +30+0 "$label" \
        "$stem.label.png"
}

echo "== shells =="
pane zsh  "$DEMO_THEME"      "zsh"
pane bash "$DEMO_THEME"      "bash"
pane fish "$DEMO_THEME"      "fish"
"$MAGICK" "$work/pane-zsh.label.png" "$work/pane-bash.label.png" "$work/pane-fish.label.png" \
    -background '#121314' -gravity center -append "$images/shells.png"

echo "== presets =="
pane zsh "--preset lean"    "lean"
pane zsh "--preset classic" "classic"
pane zsh "--preset rainbow" "rainbow"
pane zsh "--preset pure"    "pure"
"$MAGICK" "$work/pane-lean.label.png" "$work/pane-classic.label.png" \
    "$work/pane-rainbow.label.png" "$work/pane-pure.label.png" \
    -background '#121314' -gravity center -append "$images/presets.png"

echo "== done =="
ls -l "$images"
