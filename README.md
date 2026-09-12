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

## PlantUML and Mermaid

Markdown `plantuml`, `puml`, and `uml` fenced blocks render automatically in Reading
view, as do `mermaid` and `mmd` fenced blocks.

Org follows Babel semantics: add a `:file` result and press `C-c C-c` with the caret inside the source block. Org Studio writes SVG, PNG, or PDF according to the filename extension and inserts or replaces the `#+RESULTS` file link as an undoable document edit. The same workflow applies to `plantuml` and `mermaid` source blocks.

```org
#+begin_src plantuml :file images/login.svg
@startuml
Alice -> Bob: Login
@enduml
#+end_src
```

```org
#+begin_src mermaid :file images/flow.svg
flowchart TB; A[Start] --> B[End]
#+end_src
```

Mermaid support tracks TypstUML's subset: `flowchart`/`graph` diagrams only. Other
Mermaid diagram types and unsupported statements surface as inline render diagnostics
rather than silent failures.

## Typst source blocks

Org `typst` source blocks use the same Babel workflow. Set a `.svg`, `.png`, or `.pdf`
`:file`, then press the block run button or `C-c C-c`. Compilation uses Org Studio's
embedded Typst runtime, system fonts, and the Org document directory as its sandboxed
resource root.

```org
#+begin_src typst :file images/card.svg
#set page(width: 320pt, height: 180pt, margin: 20pt)
#rect(fill: luma(240), radius: 8pt, inset: 16pt)[Org Studio]
#+end_src
```
