#!/usr/bin/env bats
# The fish reference frame: capture the prompt in a real terminal and compare the pixels.

load helper

@test "fish reference frame" {
  capture_and_compare fish
}
