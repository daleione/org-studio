use super::{WorkflowError, atomic_write, relevel_subtree};
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CaptureTemplate {
    #[default]
    Task,
    Note,
    Meeting,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CaptureDraft {
    pub(crate) template: CaptureTemplate,
    pub(crate) title: String,
    pub(crate) notes: String,
    pub(crate) todo: String,
    pub(crate) scheduled: Option<String>,
}

impl CaptureDraft {
    pub(crate) fn render_org(&self) -> Result<String, WorkflowError> {
        let title = self.title.trim();
        if title.is_empty() || title.contains('\n') {
            return Err(WorkflowError::InvalidInput("capture title"));
        }
        let todo = if self.todo.trim().is_empty() {
            "TODO"
        } else {
            self.todo.trim()
        };
        if todo.contains(char::is_whitespace) {
            return Err(WorkflowError::InvalidInput("capture TODO state"));
        }
        let tags = match self.template {
            CaptureTemplate::Task => "",
            CaptureTemplate::Note => " :note:",
            CaptureTemplate::Meeting => " :meeting:",
        };
        let mut result = format!("* {todo} {title}{tags}\n");
        if let Some(date) = self
            .scheduled
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            result.push_str(&format!("SCHEDULED: <{}>\n", date.trim()));
        }
        if !self.notes.trim().is_empty() {
            result.push_str(self.notes.trim());
            result.push('\n');
        }
        Ok(result)
    }
}

pub(crate) fn append_capture(
    path: &Path,
    heading: Option<&str>,
    draft: &CaptureDraft,
) -> Result<(), WorkflowError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let source = capture_text(source, heading, draft)?;
    atomic_write(path, source.as_bytes())?;
    Ok(())
}

pub(crate) fn capture_text(
    mut source: String,
    heading: Option<&str>,
    draft: &CaptureDraft,
) -> Result<String, WorkflowError> {
    let mut entry = draft.render_org()?;
    let target = heading
        .filter(|heading| !heading.trim().is_empty())
        .and_then(|heading| find_heading_end(&source, heading));
    let insertion = target.map(|value| value.0).unwrap_or(source.len());
    if let Some((_, level)) = target {
        entry = relevel_subtree(&entry, 1, level as u16 + 1);
    }
    let prefix = if insertion > 0 && !source[..insertion].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let suffix = if insertion < source.len() && !entry.ends_with("\n\n") {
        "\n"
    } else {
        ""
    };
    source.insert_str(insertion, &format!("{prefix}{entry}{suffix}"));
    Ok(source)
}

fn find_heading_end(source: &str, title: &str) -> Option<(usize, usize)> {
    let mut found = None;
    let mut level = 0usize;
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        let stars = line.bytes().take_while(|byte| *byte == b'*').count();
        if found.is_none() && stars > 0 {
            let content = line[stars..].trim();
            if content == title || content.ends_with(&format!(" {title}")) {
                found = Some(offset + line.len());
                level = stars;
            }
        } else if found.is_some() && stars > 0 && stars <= level {
            return Some((offset, level));
        }
        offset += line.len();
    }
    found.map(|_| (source.len(), level))
}
