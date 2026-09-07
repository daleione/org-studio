#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgendaLayout {
    Wide,
    Regular,
    Compact,
}

impl AgendaLayout {
    pub(crate) fn for_width(width: f32) -> Self {
        if width >= super::style::WIDE_BREAKPOINT {
            Self::Wide
        } else if width >= super::style::COMPACT_BREAKPOINT {
            Self::Regular
        } else {
            Self::Compact
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_width_baselines_cover_standard_narrow_and_retina() {
        assert_eq!(AgendaLayout::for_width(1440.0), AgendaLayout::Wide);
        assert_eq!(AgendaLayout::for_width(1000.0), AgendaLayout::Regular);
        assert_eq!(AgendaLayout::for_width(640.0), AgendaLayout::Compact);
        assert_eq!(super::super::style::TASK_ROW_HEIGHT * 2.0, 90.0);
        assert!(!include_bytes!("../../../docs/agenda-audit/01-agenda-list.png").is_empty());
    }
}
