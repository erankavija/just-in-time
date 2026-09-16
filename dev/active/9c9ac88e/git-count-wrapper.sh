#!/bin/sh
printf '%s\n' "$*" >> "$GIT_COUNT_LOG"
exec /usr/bin/git "$@"
