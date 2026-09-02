use std::time::Duration;

pub(crate) const FOLD_ANIMATION_DURATION: Duration = Duration::from_millis(160);

pub(crate) fn ease_out_cubic(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    1.0 - (1.0 - progress).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_keeps_transition_endpoints_exact() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}
