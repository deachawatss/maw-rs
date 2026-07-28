---
pattern: Fake command processes must drain piped stdin when the production runner writes it.
date: 2026-07-28
source: rrr: github.com/deachawatss/maw-rs
concepts: [testing, fixtures, subprocess, ci, reliability]
---

# Fake stdin must be drained

When a production subprocess runner opens a pipe and calls `write_all`, a test
fake that returns success without reading stdin is not inert. It races the
writer: under scheduling pressure the child can exit first and the writer gets
EPIPE. Model the input contract in the fake by draining stdin, then assert the
user-visible result remains the successful path. If a fixture failure has a
graceful fallback, include stderr in the assertion so the lost delivery reason
survives CI logs.
