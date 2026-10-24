#!/bin/sh
# Fails on a high or critical advisory against an installed npm package. Run
# from app/ by `just check` and ci's lint job, never by a commit hook: the
# answer depends on today's advisory database, not on what is being committed.
#
# Each exception says why it is safe and when it goes.
#
# GHSA-p6vx-979v-rg4c, GHSA-jp82-f5mq-hwhp: seroval, Solid's server-side
# serialiser. Only solid-js/web/dist/server.js imports it and the app renders
# in the client only; `grep -ci seroval dist` after `bun run vite:build` is 0.
# Goes with solid-js 1.9.16 or later, which moves to seroval ~1.6.8.
#
# Not an ignore but fixed for this audit: app/package.json's `overrides` puts
# dompurify at 3.4.17 under monaco-editor 0.56.0, which pins 3.4.8 exactly
# (GHSA-cg2g-xh64-v4q7 and others). Goes once monaco-editor pins 3.4.17 or
# later itself; 0.57.0 still pins 3.4.15.
exec bun audit --audit-level=high \
	--ignore=GHSA-p6vx-979v-rg4c \
	--ignore=GHSA-jp82-f5mq-hwhp
