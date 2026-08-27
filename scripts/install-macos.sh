#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir=${ORG_STUDIO_APP_DIR:-"$HOME/Applications"}
bin_dir=${ORG_STUDIO_BIN_DIR:-"$HOME/.local/bin"}
source_app=$("$project_dir/scripts/package-macos.sh")
installed_app="$app_dir/Org Studio.app"

mkdir -p "$app_dir" "$bin_dir"
ditto "$source_app" "$installed_app"

launcher="$bin_dir/org-studio"
escaped_app=$(printf '%s' "$installed_app" | sed "s/'/'\\\\''/g")
# `--args` puts document paths in argv before GPUI creates its first window;
# plain `--` delivers them later through the open-URLs callback and flashes the
# Empty state. `-n` guarantees those argv reach a fresh process, while `open`
# remains a detached launcher and immediately returns control to the terminal.
printf '%s\n' \
    '#!/bin/sh' \
    'set -eu' \
    'if [ "$#" -gt 0 ]; then' \
    '    case "$1" in' \
    '        /*) document=$1 ;;' \
    '        *) document=$(CDPATH= cd -- "$(dirname -- "$1")" && pwd -P)/$(basename -- "$1") ;;' \
    '    esac' \
    '    shift' \
    '    set -- "$document" "$@"' \
    'fi' \
    "exec /usr/bin/open -n -a '$escaped_app' --args \"\$@\"" > "$launcher"
chmod 755 "$launcher"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$installed_app"

printf 'Installed App: %s\n' "$installed_app"
printf 'Installed CLI: %s\n' "$launcher"
printf 'Run: org-studio path/to/file.org\n'
