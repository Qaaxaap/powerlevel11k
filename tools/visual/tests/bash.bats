#!/usr/bin/env bats
# The bash frames: the prompt in a real terminal at three widths, plus the dirty scene.

load helper

@test "bash reference frame at 700x450" {
  capture_and_compare bash reference 700x450 bash-700x450
}

@test "bash reference frame at 560x450" {
  capture_and_compare bash reference 560x450 bash-560x450
}

@test "bash reference frame at 900x450" {
  capture_and_compare bash reference 900x450 bash-900x450
}

@test "bash dirty frame at 800x450" {
  capture_and_compare bash dirty 800x450 bash-dirty
}
