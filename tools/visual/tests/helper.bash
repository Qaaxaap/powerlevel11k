#!/usr/bin/env bash
# bats helpers for the render check. Each shell's .bats file loads this and calls
# capture_and_compare.

# Capture the reference prompt in one shell and compare the frame with its committed reference.
# Failure diagnostics go to file descriptor 3, which bats reports with the test.
capture_and_compare() {
  local shell=$1
  local out=${P11K_VISUAL_OUT:?P11K_VISUAL_OUT is not set}
  local expected=${P11K_VISUAL_EXPECTED:?P11K_VISUAL_EXPECTED is not set}
  local run="$out/run-$shell.sh"
  local log="$out/$shell/capture.log"

  [ -f "$run" ] || {
    echo "missing launcher: $run (run prepare.sh first)" >&3
    return 1
  }
  mkdir -p "$out/$shell"

  if ! bash "${P11K_VISUAL_RECORD_TUI:?P11K_VISUAL_RECORD_TUI is not set}" \
    --out "$out/$shell" \
    --kitty-conf "${P11K_VISUAL_KITTY_CONF:?P11K_VISUAL_KITTY_CONF is not set}" \
    --min-wait 3 \
    --title "p11k-$shell" \
    --type "sleep 1" \
    --type-wait 2 \
    -- "$run" > "$log" 2>&1; then
    echo "record-tui failed for $shell:" >&3
    sed 's/^/  /' "$log" >&3
    return 1
  fi

  if [ "${P11K_VISUAL_UPDATE:-0}" = "1" ]; then
    bash "${BATS_TEST_DIRNAME}/../compare.sh" --out "$out" --expected "$expected" --shells "$shell" --update
  else
    bash "${BATS_TEST_DIRNAME}/../compare.sh" --out "$out" --expected "$expected" --shells "$shell"
  fi
}
