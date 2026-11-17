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
#
# Ignored: GHSA-fc5q-39xc-vjpf, a ReDoS in picomatch through 4.0.7 with no
# fixed release to move to. picomatch is only in vite's and vitest's tree,
# never in the app, where it compiles the globs our own config names; the
# attack needs the glob itself to be the attacker's. Goes once picomatch
# publishes a fix.
exec bun audit --audit-level=high --ignore=GHSA-fc5q-39xc-vjpf
