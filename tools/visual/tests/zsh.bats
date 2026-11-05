#!/usr/bin/env bats
# The zsh reference frame: capture the prompt in a real terminal and compare the pixels.

load helper

@test "zsh reference frame" {
  capture_and_compare zsh
}
