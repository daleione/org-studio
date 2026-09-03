# Org Studio

## macOS installation

```sh
./scripts/install-macos.sh
```

The default installation locations are:

- `~/Applications/Org Studio.app`
- `~/.local/bin/org-studio`

Make sure `~/.local/bin` is in `PATH`, then open Org or Markdown documents with:

```sh
org-studio notes.org
org-studio README.md
```

Finder, drag-and-drop, and `open` use the same macOS Open Document event path. Override installation locations with `ORG_STUDIO_APP_DIR` and `ORG_STUDIO_BIN_DIR`.

## PlantUML

Markdown `plantuml`, `puml`, and `uml` fenced blocks render automatically in Reading view.

Org follows Babel semantics: add a `:file` result and press `C-c C-c` with the caret inside the source block. Org Studio writes SVG, PNG, or PDF according to the filename extension and inserts or replaces the `#+RESULTS` file link as an undoable document edit.

```org
#+begin_src plantuml :file images/login.svg
@startuml
Alice -> Bob: Login
@enduml
#+end_src
```
