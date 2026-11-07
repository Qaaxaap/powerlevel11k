#!/usr/bin/env bats
# The pwsh frames: the prompt in a real terminal at three widths, plus the dirty scene.

load helper

@test "pwsh reference frame at 700x450" {
  capture_and_compare pwsh reference 700x450 pwsh-700x450
}

@test "pwsh reference frame at 560x450" {
  capture_and_compare pwsh reference 560x450 pwsh-560x450
}

@test "pwsh reference frame at 900x450" {
  capture_and_compare pwsh reference 900x450 pwsh-900x450
}

@test "pwsh dirty frame at 800x450" {
  capture_and_compare pwsh dirty 800x450 pwsh-dirty
}
