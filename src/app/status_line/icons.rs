use gpui::{IntoElement, PathBuilder, Styled, canvas, point, px, rgb};

#[derive(Clone, Copy)]
pub(super) enum StatusIcon {
    Document,
    Lines,
    Storage,
    Position,
    Outline,
    Format,
}

pub(super) fn status_icon(icon: StatusIcon) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let paths: &[&[(f32, f32)]] = match icon {
                StatusIcon::Document => &[
                    &[
                        (3., 1.),
                        (9., 1.),
                        (12., 4.),
                        (12., 14.),
                        (3., 14.),
                        (3., 1.),
                    ],
                    &[(9., 1.), (9., 4.), (12., 4.)],
                    &[(5., 7.), (10., 7.)],
                    &[(5., 10.), (9., 10.)],
                ],
                StatusIcon::Lines => &[
                    &[(2., 3.), (3., 3.)],
                    &[(6., 3.), (13., 3.)],
                    &[(2., 7.), (3., 7.)],
                    &[(6., 7.), (13., 7.)],
                    &[(2., 11.), (3., 11.)],
                    &[(6., 11.), (13., 11.)],
                ],
                StatusIcon::Storage => &[
                    &[
                        (2., 3.),
                        (4., 1.5),
                        (11., 1.5),
                        (13., 3.),
                        (11., 4.5),
                        (4., 4.5),
                        (2., 3.),
                        (2., 12.),
                        (4., 13.5),
                        (11., 13.5),
                        (13., 12.),
                        (13., 3.),
                    ],
                    &[(2., 7.), (4., 8.5), (11., 8.5), (13., 7.)],
                    &[(2., 10.), (4., 11.5), (11., 11.5), (13., 10.)],
                ],
                StatusIcon::Position => &[
                    &[
                        (7.5, 2.5),
                        (11., 4.),
                        (12.5, 7.5),
                        (11., 11.),
                        (7.5, 12.5),
                        (4., 11.),
                        (2.5, 7.5),
                        (4., 4.),
                        (7.5, 2.5),
                    ],
                    &[(7.5, 0.), (7.5, 5.)],
                    &[(7.5, 10.), (7.5, 15.)],
                    &[(0., 7.5), (5., 7.5)],
                    &[(10., 7.5), (15., 7.5)],
                ],
                StatusIcon::Outline => &[
                    &[(1., 5.), (7.5, 1.), (14., 5.), (7.5, 9.), (1., 5.)],
                    &[(1., 8.), (7.5, 12.), (14., 8.)],
                    &[(1., 11.), (7.5, 15.), (14., 11.)],
                ],
                StatusIcon::Format => &[
                    &[
                        (2., 1.),
                        (10., 1.),
                        (13., 4.),
                        (13., 14.),
                        (2., 14.),
                        (2., 1.),
                    ],
                    &[(4., 10.), (6., 5.), (8., 10.)],
                    &[(5., 8.), (7., 8.)],
                    &[(9., 7.), (11., 7.)],
                    &[(10., 6.), (10., 11.)],
                ],
            };
            let mut path = PathBuilder::stroke(px(1.0));
            for points in paths {
                for (index, &(x, y)) in points.iter().enumerate() {
                    let position = point(bounds.origin.x + px(x), bounds.origin.y + px(y));
                    if index == 0 {
                        path.move_to(position);
                    } else {
                        path.line_to(position);
                    }
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(super::STATUS_FOREGROUND));
            }
        },
    )
    .size(px(15.0))
    .flex_none()
}
