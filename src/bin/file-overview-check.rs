fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/private/tmp/org-studio-file-overview-check"));
    org_studio::app::run_visual_check(output);
}
