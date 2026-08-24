#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeadingParts {
    pub todo: Option<String>,
    pub priority: Option<char>,
    pub title: String,
    pub cookie: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CheckboxState { Empty, Partial, Checked }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListParts {
    pub indent: String,
    pub marker: String,
    pub counter: Option<String>,
    pub checkbox: Option<CheckboxState>,
    pub term: Option<String>,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlanningParts { pub entries: Vec<(String, String)> }

impl HeadingParts {
    pub(crate) fn native_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(todo) = &self.todo { parts.push(todo.clone()); }
        if let Some(priority) = self.priority { parts.push(format!("[#{priority}]")); }
        if !self.title.is_empty() { parts.push(self.title.clone()); }
        if let Some(cookie) = &self.cookie { parts.push(cookie.clone()); }
        if !self.tags.is_empty() { parts.push(format!(":{}:", self.tags.join(":"))); }
        parts.join(" ")
    }
}

impl ListParts {
    pub(crate) fn native_text(&self) -> String {
        let mut result = format!("{}{}", self.indent, self.marker);
        if let Some(counter) = &self.counter { result.push_str(&format!(" {counter}")); }
        if let Some(checkbox) = &self.checkbox {
            result.push_str(match checkbox {
                CheckboxState::Empty => " [ ]",
                CheckboxState::Partial => " [-]",
                CheckboxState::Checked => " [X]",
            });
        }
        result.push(' ');
        if let Some(term) = &self.term { result.push_str(&format!("{term} :: ")); }
        result.push_str(&self.body);
        result
    }
}

impl PlanningParts {
    pub(crate) fn native_text(&self) -> String {
        self.entries.iter().map(|(key, value)| format!("{key}: {value}")).collect::<Vec<_>>().join(" ")
    }
}

pub(crate) fn parse_heading(text: &str) -> HeadingParts {
    let (body, tags) = split_tags(text.trim());
    let mut words = body.split_whitespace().peekable();
    let todo = words.peek().filter(|word| is_todo_keyword(word)).map(|word| (*word).to_owned());
    if todo.is_some() { words.next(); }
    let priority = words.peek().and_then(|word| parse_priority(word));
    if priority.is_some() { words.next(); }
    let mut remaining = words.collect::<Vec<_>>();
    let cookie = remaining.last().filter(|word| is_statistics_cookie(word)).map(|word| (*word).to_owned());
    if cookie.is_some() { remaining.pop(); }
    HeadingParts { todo, priority, title: remaining.join(" "), cookie, tags }
}

pub(crate) fn parse_list_item(text: &str) -> ListParts {
    let indent_len = text.len() - text.trim_start_matches([' ', '\t']).len();
    let indent = text[..indent_len].to_owned();
    let rest = &text[indent_len..];
    let marker_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let marker = rest[..marker_end].to_owned();
    let mut body = rest[marker_end..].trim_start();
    let counter = take_bracket_token(body, "[@").map(str::to_owned);
    if let Some(token) = &counter { body = body[token.len()..].trim_start(); }
    let checkbox = match body.get(..3) {
        Some("[ ]") => Some(CheckboxState::Empty),
        Some("[-]") => Some(CheckboxState::Partial),
        Some("[X]") | Some("[x]") => Some(CheckboxState::Checked),
        _ => None,
    };
    if checkbox.is_some() { body = body[3..].trim_start(); }
    let (term, body) = body.split_once(" :: ")
        .map(|(term, description)| (Some(term.to_owned()), description.to_owned()))
        .unwrap_or((None, body.to_owned()));
    ListParts { indent, marker, counter, checkbox, term, body }
}

pub(crate) fn parse_planning(text: &str) -> PlanningParts {
    const KEYS: [&str; 3] = ["SCHEDULED:", "DEADLINE:", "CLOSED:"];
    let mut entries = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(key) = KEYS.iter().find(|key| rest.starts_with(**key)) else { break; };
        rest = rest[key.len()..].trim_start();
        let next = KEYS.iter().filter_map(|candidate| rest.find(candidate)).min().unwrap_or(rest.len());
        entries.push((key.trim_end_matches(':').to_string(), rest[..next].trim().to_string()));
        rest = rest[next..].trim_start();
    }
    PlanningParts { entries }
}

pub(crate) fn parse_drawer_property(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix(':')?;
    let separator = rest.find(':')?;
    let key = &rest[..separator];
    if key.is_empty() || !key.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')) { return None; }
    Some((key, rest[separator + 1..].trim_start()))
}

fn split_tags(text: &str) -> (&str, Vec<String>) {
    let Some(start) = text.rfind(char::is_whitespace) else { return (text, Vec::new()); };
    let candidate = text[start..].trim();
    if candidate.len() < 3 || !candidate.starts_with(':') || !candidate.ends_with(':') { return (text, Vec::new()); }
    let tags = candidate[1..candidate.len() - 1].split(':').filter(|tag| !tag.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    if tags.is_empty() { (text, Vec::new()) } else { (text[..start].trim_end(), tags) }
}

fn is_todo_keyword(word: &&str) -> bool {
    matches!(*word, "TODO" | "DONE")
}

fn parse_priority(word: &&str) -> Option<char> {
    let bytes = word.as_bytes();
    let priority = (bytes.len() == 4).then(|| bytes[2] as char)?;
    (bytes[0] == b'[' && bytes[1] == b'#' && bytes[3] == b']' && priority.is_ascii_alphabetic())
        .then_some(priority)
}

fn is_statistics_cookie(word: &&str) -> bool {
    let Some(inner) = word.strip_prefix('[').and_then(|word| word.strip_suffix(']')) else { return false; };
    (inner.ends_with('%') && inner[..inner.len() - 1].chars().all(|ch| ch.is_ascii_digit()))
        || inner.split_once('/').is_some_and(|(done, total)| !done.is_empty() && !total.is_empty() && done.chars().all(|ch| ch.is_ascii_digit()) && total.chars().all(|ch| ch.is_ascii_digit()))
}

fn take_bracket_token<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix(prefix)?;
    let end = text.find(']')?;
    Some(&text[..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_metadata_without_losing_title() {
        assert_eq!(parse_heading("TODO [#A] Ship preview [3/7] :ui:mac:"), HeadingParts {
            todo: Some("TODO".into()), priority: Some('A'), title: "Ship preview".into(),
            cookie: Some("[3/7]".into()), tags: vec!["ui".into(), "mac".into()],
        });
    }

    #[test]
    fn parses_native_list_extensions() {
        assert_eq!(parse_list_item("  3. [@8] [-] parser :: keep Org text"), ListParts {
            indent: "  ".into(), marker: "3.".into(), counter: Some("[@8]".into()),
            checkbox: Some(CheckboxState::Partial), term: Some("parser".into()), body: "keep Org text".into(),
        });
    }

    #[test]
    fn parses_multiple_planning_fields() {
        assert_eq!(parse_planning("SCHEDULED: <2026-08-25 Tue> DEADLINE: <2026-08-28 Fri>").entries,
            vec![("SCHEDULED".into(), "<2026-08-25 Tue>".into()), ("DEADLINE".into(), "<2026-08-28 Fri>".into())]);
    }

    #[test]
    fn parses_drawer_property() {
        assert_eq!(parse_drawer_property(":OWNER: Ada"), Some(("OWNER", "Ada")));
        assert_eq!(parse_drawer_property("plain text"), None);
    }

    #[test]
    fn native_text_round_trips_supported_semantics() {
        let heading = "TODO [#A] Ship preview [3/7] :ui:mac:";
        let list = "  3. [@8] [-] parser :: keep Org text";
        let planning = "SCHEDULED: <2026-08-25 Tue> DEADLINE: <2026-08-28 Fri>";
        assert_eq!(parse_heading(heading).native_text(), heading);
        assert_eq!(parse_list_item(list).native_text(), list);
        assert_eq!(parse_planning(planning).native_text(), planning);
    }

    #[test]
    fn does_not_treat_uppercase_title_words_as_todo_keywords() {
        let heading = parse_heading("API Design");
        assert_eq!(heading.todo, None);
        assert_eq!(heading.title, "API Design");
    }

    #[test]
    fn rejects_non_alphabetic_priority() {
        let heading = parse_heading("TODO [#1] Keep literal priority");
        assert_eq!(heading.priority, None);
        assert_eq!(heading.title, "[#1] Keep literal priority");
    }
}
