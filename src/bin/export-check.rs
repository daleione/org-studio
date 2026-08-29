use std::{env, fs, path::PathBuf};

use org_studio::{
    document::RopeSnapshot,
    export::{ExportOptions, ExportSourceFormat, export_snapshot, shared_engine},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for argument in env::args_os().skip(1) {
        let path = PathBuf::from(argument);
        let format = match path.extension().and_then(|extension| extension.to_str()) {
            Some("org") => ExportSourceFormat::Org,
            Some("md" | "markdown") => ExportSourceFormat::Markdown,
            _ => continue,
        };
        let snapshot = RopeSnapshot::from_utf8(fs::read(&path)?)?;
        match export_snapshot(
            shared_engine(),
            &snapshot,
            format,
            &path,
            &ExportOptions::default(),
        ) {
            Ok(output) => {
                println!(
                    "ok: {} ({} warnings)",
                    path.display(),
                    output.diagnostics.len()
                );
                for diagnostic in output.diagnostics {
                    println!("  [{}] {}", diagnostic.code, diagnostic.message);
                }
            }
            Err(error) => println!("error: {}: {error}", path.display()),
        }
    }
    Ok(())
}
