pub(crate) fn agenda_icon(path: &str) -> &'static [u8] {
    match path {
        "assets/icons/agenda/caret-left.svg" => {
            include_bytes!("../../../assets/icons/agenda/caret-left.svg")
        }
        "assets/icons/agenda/caret-right.svg" => {
            include_bytes!("../../../assets/icons/agenda/caret-right.svg")
        }
        "assets/icons/agenda/funnel.svg" => {
            include_bytes!("../../../assets/icons/agenda/funnel.svg")
        }
        "assets/icons/agenda/phosphor-calendar.svg" => {
            include_bytes!("../../../assets/icons/agenda/phosphor-calendar.svg")
        }
        "assets/icons/agenda/calendar.svg" => {
            include_bytes!("../../../assets/icons/agenda/calendar.svg")
        }
        "assets/icons/agenda/clock.svg" => include_bytes!("../../../assets/icons/agenda/clock.svg"),
        "assets/icons/agenda/arrow-circle-right.svg" => {
            include_bytes!("../../../assets/icons/agenda/arrow-circle-right.svg")
        }
        "assets/icons/agenda/hourglass.svg" => {
            include_bytes!("../../../assets/icons/agenda/hourglass.svg")
        }
        "assets/icons/agenda/calendar-slash.svg" => {
            include_bytes!("../../../assets/icons/agenda/calendar-slash.svg")
        }
        "assets/icons/agenda/search.svg" => {
            include_bytes!("../../../assets/icons/agenda/search.svg")
        }
        "assets/icons/agenda/tray.svg" => include_bytes!("../../../assets/icons/agenda/tray.svg"),
        "assets/icons/agenda/check-circle.svg" => {
            include_bytes!("../../../assets/icons/agenda/check-circle.svg")
        }
        "assets/icons/agenda/tree-structure.svg" => {
            include_bytes!("../../../assets/icons/agenda/tree-structure.svg")
        }
        "assets/icons/agenda/flag.svg" => include_bytes!("../../../assets/icons/agenda/flag.svg"),
        "assets/icons/agenda/warning-circle.svg" => {
            include_bytes!("../../../assets/icons/agenda/warning-circle.svg")
        }
        "assets/icons/agenda/trend-up.svg" => {
            include_bytes!("../../../assets/icons/agenda/trend-up.svg")
        }
        "assets/icons/agenda/star.svg" => include_bytes!("../../../assets/icons/agenda/star.svg"),
        "assets/icons/agenda/briefcase.svg" => {
            include_bytes!("../../../assets/icons/agenda/briefcase.svg")
        }
        "assets/icons/agenda/arrows-clockwise.svg" => {
            include_bytes!("../../../assets/icons/agenda/arrows-clockwise.svg")
        }
        "assets/icons/agenda/gear.svg" => include_bytes!("../../../assets/icons/agenda/gear.svg"),
        "assets/icons/agenda/question.svg" => {
            include_bytes!("../../../assets/icons/agenda/question.svg")
        }
        "assets/icons/agenda/calendar-blank.svg" => {
            include_bytes!("../../../assets/icons/agenda/calendar-blank.svg")
        }
        "assets/icons/agenda/list-bullets.svg" => {
            include_bytes!("../../../assets/icons/agenda/list-bullets.svg")
        }
        "assets/icons/agenda/calendar-dots.svg" => {
            include_bytes!("../../../assets/icons/agenda/calendar-dots.svg")
        }
        "assets/icons/agenda/file-code.svg" => {
            include_bytes!("../../../assets/icons/agenda/file-code.svg")
        }
        "assets/icons/agenda/plus.svg" => include_bytes!("../../../assets/icons/agenda/plus.svg"),
        "assets/icons/agenda/dots-three-vertical.svg" => {
            include_bytes!("../../../assets/icons/agenda/dots-three-vertical.svg")
        }
        "assets/icons/agenda/x.svg" => include_bytes!("../../../assets/icons/agenda/x.svg"),
        "assets/icons/agenda/arrow-square-out.svg" => {
            include_bytes!("../../../assets/icons/agenda/arrow-square-out.svg")
        }
        "assets/icons/agenda/file-text.svg" => {
            include_bytes!("../../../assets/icons/agenda/file-text.svg")
        }
        _ => include_bytes!("../../../assets/icons/agenda/calendar.svg"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_reuse_embedded_bytes_without_a_heap_cache() {
        let first = agenda_icon("assets/icons/agenda/search.svg");
        let second = agenda_icon("assets/icons/agenda/search.svg");
        assert!(std::ptr::eq(first, second));
        assert!(std::str::from_utf8(first).unwrap().contains("<svg"));
        assert!(!agenda_icon("assets/icons/agenda/calendar.svg").is_empty());
        assert!(
            include_str!("../../../assets/icons/agenda/LICENSE.md")
                .contains("original project assets")
        );
    }
}
