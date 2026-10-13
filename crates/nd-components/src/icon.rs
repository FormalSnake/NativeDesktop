//! Icons by Lucide name. Backends map the name to the platform set (SF Symbols on macOS,
//! Segoe Fluent on Windows) and fall back to the Lucide asset (`nd-icons`).

/// The icons nd names today. Each has a Lucide name, which is what crosses to backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Archive,
    Check,
    ChevronLeft,
    ChevronRight,
    CircleCheck,
    FileText,
    Folder,
    Inbox,
    Info,
    MapPin,
    Minus,
    Notebook,
    PanelLeft,
    PanelRight,
    Pin,
    Plus,
    Search,
    Settings,
    Share,
    SquarePen,
    Star,
    Trash,
    TriangleAlert,
}

impl Icon {
    pub fn lucide(self) -> &'static str {
        match self {
            Icon::Archive => "archive",
            Icon::Check => "check",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::CircleCheck => "circle-check",
            Icon::FileText => "file-text",
            Icon::Folder => "folder",
            Icon::Inbox => "inbox",
            Icon::Info => "info",
            Icon::MapPin => "map-pin",
            Icon::Minus => "minus",
            Icon::Notebook => "notebook",
            Icon::PanelLeft => "panel-left",
            Icon::PanelRight => "panel-right",
            Icon::Pin => "pin",
            Icon::Plus => "plus",
            Icon::Search => "search",
            Icon::Settings => "settings",
            Icon::Share => "share",
            Icon::SquarePen => "square-pen",
            Icon::Star => "star",
            Icon::Trash => "trash",
            Icon::TriangleAlert => "triangle-alert",
        }
    }
}
