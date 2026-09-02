pub(crate) fn fence_start(line: &str) -> Option<(char, usize, Option<String>)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = line.chars().take_while(|ch| *ch == marker).count();
    if count < 3 {
        return None;
    }
    let marker_bytes = marker.len_utf8() * count;
    let info = line[marker_bytes..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let language = info
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some((marker, count, language))
}

pub(crate) fn is_closing_fence(line: &str, marker: char, opening_count: usize) -> bool {
    let count = line.chars().take_while(|ch| *ch == marker).count();
    count >= opening_count && line[marker.len_utf8() * count..].trim().is_empty()
}

pub(crate) fn atx_heading(line: &str) -> Option<(u16, usize)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&count)
        || line
            .as_bytes()
            .get(count)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    let rest = line[count..].trim_start();
    Some((count as u16, line.len() - rest.len()))
}
