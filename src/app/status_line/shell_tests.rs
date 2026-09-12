use super::*;

fn request(kind: ShellKind, width: f32, height: f32) -> ShellRequest {
    ShellRequest {
        kind,
        pane: PaneSide::Left,
        available: 1000.,
        returning: false,
        target: ShellShape {
            width,
            height,
            expansion_height: 30.,
            content_opacity: 1.,
        },
    }
}
fn returning(mut request: ShellRequest) -> ShellRequest {
    request.returning = true;
    request.target = ShellShape::status(request.available);
    request
}

#[test]
fn interrupted_animation_keeps_all_dimensions_continuous() {
    let start = Instant::now();
    let mut shell = ShellHost::default();
    let search = request(ShellKind::Search, 540., 92.);
    let prefix = request(ShellKind::Prefix, 740., 240.);
    let buffers = request(ShellKind::Buffers, 650., 410.);
    assert!(shell.update(&[search], start, true));
    for (milliseconds, next) in [(90, prefix), (180, returning(prefix)), (230, buffers)] {
        let now = start + Duration::from_millis(milliseconds);
        let before = shell.motion.sample(1000., now).0;
        assert!(shell.update(&[next], now, true));
        assert_eq!(
            shell.motion.sample(1000., now).0,
            before,
            "retargeting must preserve width, height, expansion and opacity"
        );
    }
    let settled = start + Duration::from_millis(700);
    assert!(!shell.update(&[buffers], settled, true));
    assert_eq!(shell.motion.sample(1000., settled).0, buffers.target);
    assert!(shell.update(&[returning(buffers)], settled, true));
    let end = start + Duration::from_millis(1100);
    assert!(!shell.update(&[returning(buffers)], end, true));
    assert_eq!(shell.motion.sample(1000., end).0, ShellShape::status(1000.));
    assert_eq!(shell.owner, None);
}

#[test]
fn active_content_replaces_outgoing_snapshots_without_restarting_motion() {
    let now = Instant::now();
    let mut shell = ShellHost::default();
    let search = request(ShellKind::Search, 540., 92.);
    let prefix = request(ShellKind::Prefix, 740., 240.);
    let buffers = request(ShellKind::Buffers, 650., 410.);
    shell.update(&[prefix, search], now, false);
    assert!(shell.owns(ShellKind::Prefix, PaneSide::Left));
    shell.update(&[returning(prefix), buffers, search], now, true);
    assert!(shell.owns(ShellKind::Buffers, PaneSide::Left));
    assert_eq!(shell.motion.sample(1000., now).0, prefix.target);
    shell.update(&[returning(buffers), search], now, true);
    assert!(shell.owns(ShellKind::Search, PaneSide::Left));
    assert_eq!(shell.motion.sample(1000., now).0, prefix.target);
    // An inactive pane starts at its own normal statusline, not the other pane's panel.
    shell.update(
        &[ShellRequest {
            pane: PaneSide::Right,
            ..buffers
        }],
        now,
        true,
    );
    assert!(shell.owns(ShellKind::Buffers, PaneSide::Right));
    assert_eq!(shell.motion.sample(1000., now).0, ShellShape::status(1000.));
}
