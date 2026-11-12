# ADR 0028 · Phase 7c: limits for the engine process

- **Status:** accepted
- **Date:** 2026-11-12

## Problem

The engine (`llama-server`) reads files that come from anyone on Hugging Face. If a malicious model exploited a bug in
llama.cpp, the attacker's code would run with the engine's permissions, which were the same as any other program of the
user. The Job Object (ADR 0003) only guaranteed that the engine dies when Locall dies.

## Decision

The engine's Job Object (`locall_system::Trabajo`) gets the limits the engine does not need to do its work:

- **`JOB_OBJECT_LIMIT_ACTIVE_PROCESS` = 1:** only one process may live in the job. The engine cannot launch other
  programs (`cmd.exe`, PowerShell…): Windows refuses to create them. This cuts the most common next step of an attack.
- **`JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION`:** a crash ends the process at once, without the Windows error dialog.
  Locall already detects an engine that exited on its own (ADR 0025).
- **Desktop limits (`JobObjectBasicUIRestrictions`):** no clipboard (read or write), no handles to other programs'
  windows, no global atoms, no changes to display or system settings, no shutting down or logging off. The engine only
  computes: it has no window.
- `KILL_ON_JOB_CLOSE` stays as before.

No new dependency: everything uses `windows-sys`, already in the project.

## Known gap

The engine is created with `std::process::Command` and put into the job right afterwards, so for a few milliseconds it
runs outside the job. During that moment it is only starting up: it has not opened any model yet, so a malicious model
cannot act in that window. Closing it completely needs launching the process directly with `CreateProcessW` (suspended,
or with `PROC_THREAD_ATTRIBUTE_JOB_LIST`); see below.

## Considered and postponed: low integrity level

Windows can run a process at **low integrity** (like browser tabs): it can read the user's files but cannot write to
almost any of them. It would suit the engine (its log is opened by Locall and handed over already open, and listening on
`127.0.0.1` keeps working). But `std::process::Command` cannot choose the integrity level: the launch would have to be
rewritten with `CreateRestrictedToken`, `SetTokenInformation` and `CreateProcessAsUserW`, which means a lot of new
`unsafe` code. The limits above give most of the protection with little risk; low integrity remains a possible future
improvement, together with closing the gap above, since both need the same direct launch.

Not adopted either: a memory limit for the job. The memory the engine needs depends on the model and the context, and a
wrong limit would kill legitimate loads; the pre-load check (part 9c) already prevents loading what does not fit.

## Tests

- A `cmd.exe` put in the job cannot launch another `cmd.exe` (with a control run outside the job, where it can).
- A job refuses a second process.
- The existing tests with the real engine (load, chat, model switch, crash detection, idle unload) pass with the limits.
