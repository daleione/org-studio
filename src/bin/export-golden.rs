use std::{fs, path::PathBuf};

use org_studio::{
    document::DocumentSnapshot,
    export::{
        ExportFormat, ExportOptions, ExportSourceFormat, LayoutMode, export_snapshot,
        export_templates, shared_engine,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_path = workspace.join("assets/export/fixtures/theme-preview.org");
    let fixture = fs::read(&fixture_path)?;
    let snapshot = DocumentSnapshot::from_utf8(fixture)?;
    let output_dir = workspace.join("assets/export/thumbnails");
    fs::create_dir_all(&output_dir)?;

    for template in export_templates() {
        let options = ExportOptions {
            format: ExportFormat::Png,
            template_id: template.id.into(),
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
        let path = output_dir.join(format!("{}.png", template.id));
        fs::write(&path, &output.files[0])?;
        println!("{}", path.display());
    }
    Ok(())
}
