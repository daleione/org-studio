use crate::document::{ByteOffset, Revision};

use super::projection::{ReadingProjection, VisualRowId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum Bias {
    Left,
    Right,
}

impl Bias {
    pub(in crate::preview) const fn for_boundary(at_end: bool) -> Self {
        if at_end { Self::Left } else { Self::Right }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) struct SourcePoint {
    pub(in crate::preview) revision: Revision,
    pub(in crate::preview) offset: ByteOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) struct VisualPoint {
    pub(in crate::preview) revision: Revision,
    pub(in crate::preview) row: VisualRowId,
    pub(in crate::preview) offset_in_row: u64,
    pub(in crate::preview) bias: Bias,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::preview) enum PointMapError {
    RevisionMismatch,
    NotRepresented,
    RowNotFound,
    OffsetOutsideRow,
}

impl ReadingProjection {
    pub(in crate::preview) fn source_to_visual(
        &self,
        point: SourcePoint,
        bias: Bias,
    ) -> Result<VisualPoint, PointMapError> {
        if point.revision != self.revision {
            return Err(PointMapError::RevisionMismatch);
        }
        for (index, visual) in self.rows.iter().enumerate() {
            let Some(row) = self.source_row(index) else {
                continue;
            };
            let range = row.content.range;
            let inside = point.offset > range.start && point.offset < range.end;
            let boundary = (point.offset == range.start && bias == Bias::Right)
                || (point.offset == range.end && bias == Bias::Left);
            if inside || boundary {
                return Ok(VisualPoint {
                    revision: self.revision,
                    row: visual.id,
                    offset_in_row: point.offset.0.saturating_sub(range.start.0),
                    bias,
                });
            }
        }
        Err(PointMapError::NotRepresented)
    }

    pub(in crate::preview) fn visual_to_source(
        &self,
        point: VisualPoint,
    ) -> Result<SourcePoint, PointMapError> {
        if point.revision != self.revision {
            return Err(PointMapError::RevisionMismatch);
        }
        let (index, _) = self
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.id == point.row)
            .ok_or(PointMapError::RowNotFound)?;
        let row = self
            .source_row(index)
            .ok_or(PointMapError::NotRepresented)?;
        let len = row
            .content
            .range
            .end
            .0
            .saturating_sub(row.content.range.start.0);
        if point.offset_in_row > len {
            return Err(PointMapError::OffsetOutsideRow);
        }
        Ok(SourcePoint {
            revision: self.revision,
            offset: ByteOffset(row.content.range.start.0 + point.offset_in_row),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::load_document;

    #[test]
    fn source_visual_round_trip_respects_boundary_bias() {
        let path = std::env::temp_dir().join(format!(
            "org-studio-coordinate-map-{}.org",
            std::process::id()
        ));
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let document = load_document(path.clone()).unwrap().into_preview();
        let _ = std::fs::remove_file(path);
        let snapshot = &document.projection;
        let source = SourcePoint {
            revision: snapshot.revision,
            offset: ByteOffset(5),
        };
        let visual = snapshot.source_to_visual(source, Bias::Left).unwrap();
        assert_eq!(snapshot.visual_to_source(visual).unwrap(), source);
        assert!(snapshot.source_to_visual(source, Bias::Right).is_err());
    }
}
