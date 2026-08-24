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
printf '%s\n' '#!/bin/sh' "exec /usr/bin/open -a '$escaped_app' -- \"\$@\"" > "$launcher"
chmod 755 "$launcher"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$installed_app"

printf 'Installed App: %s\n' "$installed_app"
printf 'Installed CLI: %s\n' "$launcher"
printf 'Run: org-studio path/to/file.org\n'
