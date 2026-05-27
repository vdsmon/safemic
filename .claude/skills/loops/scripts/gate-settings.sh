#!/bin/bash
# Thin wrapper: invoke mic-mute settings-window gate from anywhere.
# Auto-derives repo root from this script's location.
#
# Usage:
#   .claude/skills/loops/scripts/gate-settings.sh [--diff | --update | <state-name>]
#
# Default: --diff
#
# Class id: 4aa9e37f9396

set -e

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$REPO"
exec tools/settings-preview/iterate.sh "${@:---diff}"
