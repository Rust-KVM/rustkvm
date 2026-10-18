//! The Claude Desktop style frame around every page: sidebar on the left,
//! titlebar on top, content below. See docs/design-system.md, "Layout".

mod sidebar;
mod titlebar;

pub use sidebar::Sidebar;
pub use titlebar::Titlebar;
