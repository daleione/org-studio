use std::{fs, path::PathBuf};

use org_studio::{
    document::RopeSnapshot,
    export::{
        ExportFormat, ExportOptions, ExportSourceFormat, LayoutMode, export_snapshot,
        shared_engine, themes,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_path = workspace.join("assets/export/fixtures/theme-preview.org");
    let fixture = fs::read(&fixture_path)?;
    let snapshot = RopeSnapshot::from_utf8(fixture)?;
    let output_dir = workspace.join("assets/export/thumbnails");
    fs::create_dir_all(&output_dir)?;

    for theme in themes() {
        let options = ExportOptions {
            format: ExportFormat::Png,
            theme_id: theme.id.into(),
            layout: LayoutMode::Paged,
            per_page: true,
            png_ppi: 72.0,
            page_numbers: Some(false),
            ..Default::default()
        };
        let output = export_snapshot(
            shared_engine(),
            &snapshot,
            ExportSourceFormat::Org,
            &fixture_path,
            &options,
        )?;
        let path = output_dir.join(format!("{}.png", theme.id));
        fs::write(&path, &output.files[0])?;
        println!("{}", path.display());
    }
    Ok(())
}
