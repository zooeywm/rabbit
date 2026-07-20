use crate::kernel::geometry::PixelSize;

pub trait ScreenLayoutManager {
    fn refresh(&mut self) -> eros::Result<()>;
    fn screens(&self) -> &[Screen];
    fn screen(&self, id: &ScreenId) -> Option<&Screen>;
    fn primary_screen(&self) -> eros::Result<&Screen>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenId(pub(crate) u8);

impl ScreenId {
    pub const MAX: u8 = u8::MAX - 1;

    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("screen ID {0} is outside the supported range 0..={max}", max = ScreenId::MAX)]
pub struct InvalidScreenId(u8);

impl TryFrom<u8> for ScreenId {
    type Error = InvalidScreenId;

    fn try_from(id: u8) -> Result<Self, Self::Error> {
        if id > Self::MAX {
            return Err(InvalidScreenId(id));
        }

        Ok(Self(id))
    }
}

impl From<ScreenId> for u8 {
    fn from(id: ScreenId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub id: ScreenId,
    pub name: String,
    pub resolution: PixelSize,
    pub layout: ScreenLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenLayout {
    pub rect: ScreenRect,
    pub scale: f64,
    pub transform: ScreenTransform,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[cfg(test)]
mod tests {
    use crate::kernel::screen_manager::ScreenId;

    #[test]
    fn reserves_the_last_u8_value_for_transport_control_routing() {
        assert_eq!(
            ScreenId::try_from(ScreenId::MAX)
                .expect("Maximum video screen ID should be valid")
                .get(),
            254
        );
        assert!(ScreenId::try_from(u8::MAX).is_err());
    }
}
