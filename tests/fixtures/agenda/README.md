# Org Agenda semantic behavior baseline

Baseline upstream: Org Mode `main` commit `437fd11` (2026-08-04).

The fixture records the Org behavior implemented by the native semantic layer:

- file-local TODO sequences, fast keys, enter/leave logging and done boundary;
- active and inactive timestamps, time ranges and date ranges;
- `+`, `++`, and `.+` repeaters plus deadline warning periods;
- `FILETAGS`, `CATEGORY`, `PROPERTY`, and `ARCHIVE` file keywords.

Authoritative behavior references are the Org manual sections “Timestamps”,
“Deadlines and Scheduling”, “Repeated tasks”, “Fast access to TODO states”,
and “Tracking TODO state changes”. Diary sexps and arbitrary Lisp agenda skip
functions are preserved as source and deliberately outside the native parser.
