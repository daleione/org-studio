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
