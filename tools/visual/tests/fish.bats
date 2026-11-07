#!/usr/bin/env bats
# The fish frames: the prompt in a real terminal at three widths, plus the dirty scene.

load helper

@test "fish reference frame at 700x450" {
  capture_and_compare fish reference 700x450 fish-700x450
}

@test "fish reference frame at 560x450" {
  capture_and_compare fish reference 560x450 fish-560x450
}

@test "fish reference frame at 900x450" {
  capture_and_compare fish reference 900x450 fish-900x450
}

@test "fish dirty frame at 800x450" {
  capture_and_compare fish dirty 800x450 fish-dirty
}
