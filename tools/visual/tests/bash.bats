#!/usr/bin/env bats
# The bash reference frame: capture the prompt in a real terminal and compare the pixels.

load helper

@test "bash reference frame" {
  capture_and_compare bash
}
