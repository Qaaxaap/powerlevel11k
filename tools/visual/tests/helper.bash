#!/usr/bin/env bash
# bats helpers for the render check. Each shell's .bats file loads this and calls
# capture_and_compare.

# Capture one frame and compare it with its committed reference.
#
# Arguments: shell scene size frame
#   shell  zsh / bash / fish / pwsh
#   scene  reference / dirty
#   size   output resolution, e.g. 700x450
#   frame  comparison name: expected/<arch>/<frame>.png
# Failure diagnostics go to file descriptor 3, which bats reports with the test.
capture_and_compare() {
  local shell=$1
  local scene=$2
  local size=$3
  local frame=$4
  local out=${P11K_VISUAL_OUT:?P11K_VISUAL_OUT is not set}
  local expected=${P11K_VISUAL_EXPECTED:?P11K_VISUAL_EXPECTED is not set}
  local run="$out/run-$shell-$scene.sh"
  local log="$out/$frame/capture.log"
  # The command writes this marker after `sleep 1` returns; the recorder waits for it before
  # starting --type-wait, so the frame is captured after the command finished rather than while
  # it still runs.
  local marker="$out/$frame/command-done"
  rm -f "$marker"
  export P11K_VISUAL_READY="$marker"
  local command
  if [ "$shell" = "pwsh" ]; then
    command='sleep 1; echo > $env:P11K_VISUAL_READY'
  else
    command='sleep 1; echo > "$P11K_VISUAL_READY"'
  fi
  # pwsh starts slower than the other shells: give it one more second before the stability
  # check starts, so the prompt and the input echo are on screen.
  local min_wait=0.5
  if [ "$shell" = "pwsh" ]; then
    min_wait=1.5
  fi

  [ -f "$run" ] || {
    echo "missing launcher: $run (run prepare.sh first)" >&3
    return 1
  }
  mkdir -p "$out/$frame"

  if ! bash "${P11K_VISUAL_RECORD_TUI:?P11K_VISUAL_RECORD_TUI is not set}" \
    --out "$out/$frame" \
    --kitty-conf "${P11K_VISUAL_KITTY_CONF:?P11K_VISUAL_KITTY_CONF is not set}" \
    --size "$size" \
    --min-wait "$min_wait" \
    --title "p11k-$frame" \
    --type "$command" \
    --wait-for-after-type "$marker" \
    --type-wait 0.2 \
    -- "$run" > "$log" 2>&1; then
    echo "record-tui failed for $frame:" >&3
    sed 's/^/  /' "$log" >&3
    return 1
  fi

  if [ "${P11K_VISUAL_UPDATE:-0}" = "1" ]; then
    bash "${BATS_TEST_DIRNAME}/../compare.sh" --out "$out" --expected "$expected" --frames "$frame" --update
  else
    bash "${BATS_TEST_DIRNAME}/../compare.sh" --out "$out" --expected "$expected" --frames "$frame"
  fi
}
