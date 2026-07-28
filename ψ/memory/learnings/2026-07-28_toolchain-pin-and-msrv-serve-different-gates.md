---
pattern: Keep the MSRV at the proven build floor while pinning CI to the lowest concrete compiler that passes every required gate.
date: 2026-07-28
source: rrr: deachawatss/maw-rs
concepts: [rust, toolchain, msrv, ci, verification]
---

# Toolchain pin and MSRV serve different gates

A workspace can compile and test at its dependency-imposed minimum compiler
while a mandatory lint gate fails on that exact release. Record the proven build
floor in `workspace.package.rust-version`, but pin local and CI toolchains to a
concrete version that also passes formatting, tests, linting, and target builds.
Use CI evidence for the selected pin rather than suppressing pre-existing lint
diagnostics or weakening the gate.
