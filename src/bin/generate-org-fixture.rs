use std::{io::Write, path::PathBuf};

const PROSE: &str = "Org Studio keeps parsing independent from presentation so large documents remain responsive. The preview reads immutable Rope snapshots, stores compact source ranges, and resolves inline markup only when a visible block is rendered. This representative paragraph deliberately contains enough natural-language text to model notes and technical documents instead of producing an artificial node every few bytes. ";

fn template(index: u64) -> String {
    let body = PROSE.repeat(6);
    format!(
        "* Project {index}\n:PROPERTIES:\n:ID: project-{index}\n:END:\n{body} *bold*, /italic/, [[file:notes.org][a link]], and <2026-08-24 Mon>.\n\n** TODO Task {index} :work:\n- [ ] First item with enough descriptive text to represent a real task rather than a synthetic marker\n- [X] Completed item\n\n| Name | Value |\n|------+-------|\n| row  | {index}   |\n\n#+begin_src rust\n// A representative source block remains part of every section.\nfn task_{index}() {{ println!(\"preview\"); }}\n#+end_src\n\n"
    )
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("preview-benchmark.org"));
    let target_mib: u64 = args
        .next()
        .and_then(|value| value.to_string_lossy().parse().ok())
        .unwrap_or(1);
    let target_bytes = target_mib * 1024 * 1024;
    let mut file = std::io::BufWriter::new(std::fs::File::create(&output)?);
    let mut written = 0_u64;
    let mut index = 1_u64;

    while written < target_bytes {
        let chunk = template(index);
        file.write_all(chunk.as_bytes())?;
        written += chunk.len() as u64;
        index += 1;
    }
    file.flush()?;
    println!("path={} bytes={written}", output.display());
    Ok(())
}
