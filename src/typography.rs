#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentFontSize(u16);

impl ContentFontSize {
    pub const MIN: u16 = 5;
    pub const DEFAULT: u16 = 15;
    pub const MAX: u16 = 96;

    pub const fn new(value: u16) -> Self {
        if value < Self::MIN {
            Self(Self::MIN)
        } else if value > Self::MAX {
            Self(Self::MAX)
        } else {
            Self(value)
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub fn scale(self) -> f32 {
        self.0 as f32 / Self::DEFAULT as f32
    }

    pub const fn increase(self) -> Self {
        Self::new(self.0.saturating_add(1))
    }

    pub const fn decrease(self) -> Self {
        Self::new(self.0.saturating_sub(1))
    }

    pub const fn reset() -> Self {
        Self(Self::DEFAULT)
    }
}

impl Default for ContentFontSize {
    fn default() -> Self {
        Self::reset()
    }
}

#[cfg(test)]
mod tests {
    use super::ContentFontSize;

    #[test]
    fn content_font_size_clamps_steps_and_resets() {
        assert_eq!(ContentFontSize::new(0).get(), 5);
        assert_eq!(ContentFontSize::new(u16::MAX).get(), 96);
        assert_eq!(ContentFontSize::new(5).decrease().get(), 5);
        assert_eq!(ContentFontSize::new(96).increase().get(), 96);
        assert_eq!(ContentFontSize::new(15).increase().get(), 16);
        assert_eq!(ContentFontSize::new(15).decrease().get(), 14);
        assert_eq!(ContentFontSize::reset().get(), 15);
        assert_eq!(ContentFontSize::reset().scale(), 1.0);
    }
}
