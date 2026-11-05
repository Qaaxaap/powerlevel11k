#!/usr/bin/env bats
# The pwsh reference frame: capture the prompt in a real terminal and compare the pixels.

load helper

@test "pwsh reference frame" {
  capture_and_compare pwsh
}
