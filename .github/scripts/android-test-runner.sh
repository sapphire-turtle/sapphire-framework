#!/usr/bin/env bash
# A cargo runner for the Android targets: copies the test binary onto the running emulator
# and runs it there, passing cargo's arguments on. adb shell returns the binary's exit code.
set -euo pipefail
bin=$1
shift
adb push "$bin" /data/local/tmp/cargo-test >/dev/null
adb shell /data/local/tmp/cargo-test "$@"
