# ADR-0003: One process on macOS

- Status: Accepted
- Date: 2026-09-20

## Context

The PRD budgets are under 50 ms at p95 from typed abbreviation to inserted
text, and under 1 ms of work per keystroke. macOS disables an event tap that
stalls. Any hop between processes on the keystroke path spends part of that
budget and adds a way to stall.

## Decision

Aralo on macOS is one process: a menu bar agent that hosts the event tap, the
Rust core and every window.

- The matcher runs in-process with the event tap.
- There is no IPC and no async work on the keystroke path.
- The tap thread calls `engine.on_key` synchronously (see ADR-0004).

## Consequences

- The keystroke path stays free of IPC, which the 50 ms budget depends on.
- A static snippet goes from `on_key` to a finished plan without leaving the
  tap and injector threads.
- **Risk: an editor crash stops expansion.** The windows and the tap share a
  process, so a crash in UI code takes the tap down with it. The plan rates
  this low to medium.
- **Response.** The bridge API is already message-shaped: key events in,
  verdicts and plans out, all plain data. The engine and the tap can move into
  a helper process later without changing the core. Revisit this on beta crash
  data.

## Alternatives rejected

- **A separate helper process for the tap and engine from the start.** It
  would isolate expansion from UI crashes, but it puts IPC on or next to the
  keystroke path before there is evidence that crashes are a problem. The
  design keeps this door open rather than paying for it now.
