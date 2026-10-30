#!/bin/sh
# Fails on a high or critical advisory against an installed npm package. Run
# from app/ by `just check` and ci's lint job, never by a commit hook: the
# answer depends on today's advisory database, not on what is being committed.
#
# An exception, should one be needed, says why it is safe and when it goes.
#
# Not an ignore but fixed for this audit: app/package.json's `overrides` puts
# dompurify at 3.4.17 under monaco-editor 0.56.0, which pins 3.4.8 exactly
# (GHSA-cg2g-xh64-v4q7 and others). Goes once monaco-editor pins 3.4.17 or
# later itself; 0.57.0 still pins 3.4.15.
exec bun audit --audit-level=high
