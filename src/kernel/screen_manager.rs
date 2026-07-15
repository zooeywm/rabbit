pub trait ScreenLayoutManager {
    fn refresh(&mut self) -> eros::Result<()>;
    fn screens(&self) -> &[Screen];
    fn screen(&self, id: &ScreenId) -> Option<&Screen>;
    fn primary_screen(&self) -> eros::Result<&Screen>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenId(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub id: ScreenId,
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
