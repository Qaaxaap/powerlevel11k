#!/usr/bin/env bash
# bats helpers for the render check. Each shell's .bats file loads this and calls
# capture_and_compare.

# Capture one frame and compare it with its committed reference.
#
# Arguments: shell scene size frame
#   shell  zsh / bash / fish / pwsh
#   scene  reference / dirty
#   size   Output resolution, for example 700x450.
#   frame  Comparison name: expected/<arch>/<frame>.png
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
  # The command writes this marker after `sleep 1` returns. The recorder waits for the marker
  # before --type-wait starts. The capture then occurs after the command finished. It does not
  # occur while the command still runs.
  # The path is fixed and environment-independent: the same command text reaches every shell,
  # and the input line reads the same on a local run and on CI. `echo done` gives the command
  # an output object, so pwsh does not open the interactive Write-Output prompt.
  local marker="/tmp/p11k-visual-$frame-done"
  rm -f "$marker"
  local command="sleep 1; echo done > $marker"
  # Pwsh starts slower than the other shells. Type only after the first real prompt: keys that
  # arrive during the instant phase make pwsh run an empty command before the engine replaces the
  # instant header, and the capture shows the stale header above the new one. Zsh does not show
  # this because its first prompt does not start on a new row.
  local min_wait=0.7
  if [ "$shell" = "pwsh" ]; then
    min_wait=3.5
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
    --settle-before-type \
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
