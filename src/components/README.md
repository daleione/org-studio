# Date controls

`calendar::Calendar` is a reusable month grid. It accepts civil dates, a selected
range, and callbacks for selecting a date, dragging a range, previewing a date,
and changing months. It has no Org, editor, or persistence dependency. Adjacent
month grids may share `CalendarDrag`; the containing view clears this state on
mouse-up outside the grids. Explicit seven-cell rows keep weekdays aligned at
fractional pixel widths. `hide_outside_month` avoids duplicate dates in dual-month
layouts, and `navigation` controls the outer arrows.

`timestamp_picker::TimestampPicker` wraps the calendar with time, repeat, and
context-sensitive warning/delay controls. Call `timestamp_picker::init` during
application setup, create an entity with `TimestampPicker::new(source, kind,
today, language, cx)`, and subscribe to `TimestampPickerEvent::Applied(source)` or
`Cancelled`. Pass the workspace language and call `set_language` when it changes;
this updates the UI without rewriting Org source. The host owns positioning, focus restoration, revision validation,
and undoable persistence. The editor integration is in `editor/timestamp.rs`.

The picker initially shows compact date/time pills with its calendar collapsed.
Plain timestamps use single-day mode; existing date ranges retain both endpoints.
Clicking a date pill expands one month inline beneath that row. Month navigation
only changes the view; selecting a date updates that endpoint and collapses the
calendar. Time pills open a separate inline editor with a Done action. Switching
endpoints preserves their independent time ranges, repeaters, and warning rules;
invalid time text must be corrected before switching. Range mode also exposes
which endpoint the repeat and agenda settings apply to. Apply writes the whole
draft once; Cancel discards it.

`org_semantic::timestamp_edit` preserves both range endpoints independently and
patches only changed fields. Existing time ranges, repeater bounds, warning
cookies, inactive brackets, and untouched extensions survive date edits. Diary
expressions remain available through the source editor and are never evaluated.

`native_input::NativeInput` is the shared platform text field, including IME,
clipboard and undo support. `app::native_input` reexports it for existing callers.
