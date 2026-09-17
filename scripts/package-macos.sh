#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output_dir=${ORG_STUDIO_DIST_DIR:-"$project_dir/dist"}
app="$output_dir/Org Studio.app"

cargo build --manifest-path "$project_dir/Cargo.toml" --release --bin org-studio
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$project_dir/target/release/org-studio" "$app/Contents/MacOS/org-studio"
cp "$project_dir/assets/macos/Info.plist" "$app/Contents/Info.plist"
cp "$project_dir/assets/macos/OrgStudio.icns" "$app/Contents/Resources/OrgStudio.icns"
cp "$project_dir/assets/macos/OrgStudio.png" "$app/Contents/Resources/OrgStudio.png"
# GPL-3.0-or-later: a distributed binary has to ship the license text and a way to
# reach the corresponding source.
cp "$project_dir/LICENSE" "$app/Contents/Resources/LICENSE"
cp "$project_dir/assets/macos/SOURCE-OFFER.txt" "$app/Contents/Resources/SOURCE-OFFER.txt"
chmod 755 "$app/Contents/MacOS/org-studio"
# Strip only the packaged copy, preserving local symbols for profiling builds.
strip -x "$app/Contents/MacOS/org-studio"
codesign --force --deep --sign - "$app"
printf '%s\n' "$app"
