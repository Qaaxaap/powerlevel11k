#!/usr/bin/env bats
# The zsh frames: the prompt in a real terminal at three widths, plus the dirty scene.

load helper

@test "zsh reference frame at 700x450" {
  capture_and_compare zsh reference 700x450 zsh-700x450
}

@test "zsh reference frame at 560x450" {
  capture_and_compare zsh reference 560x450 zsh-560x450
}

@test "zsh reference frame at 900x450" {
  capture_and_compare zsh reference 900x450 zsh-900x450
}

@test "zsh dirty frame at 800x450" {
  capture_and_compare zsh dirty 800x450 zsh-dirty
}
