use leptos::prelude::*;

/// Line icons on a 20x20 grid, 1.5px round strokes in `currentColor`, drawn
/// for this project to match the Claude Desktop icon weight.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconName {
    Sidebar,
    Monitor,
    Terminal,
    Sliders,
    Power,
    Maximize,
    Focus,
    Keyboard,
    Mouse,
    Volume,
    VolumeOff,
    ScanText,
    Close,
    Plus,
    Minus,
    Sun,
    Moon,
    Alert,
    Languages,
}

impl IconName {
    fn path(self) -> &'static str {
        match self {
            Self::Sidebar => {
                "M5 3.5h10a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1 -2.5 2.5h-10a2.5 2.5 0 0 1 -2.5 -2.5v-8a2.5 2.5 0 0 1 2.5 -2.5zM7.5 3.5v13"
            }
            Self::Monitor => {
                "M4.5 3.5h11a2 2 0 0 1 2 2v6.5a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-6.5a2 2 0 0 1 2 -2zM7 16.5h6M10 14v2.5"
            }
            Self::Terminal => {
                "M5 3.5h10a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1 -2.5 2.5h-10a2.5 2.5 0 0 1 -2.5 -2.5v-8a2.5 2.5 0 0 1 2.5 -2.5zM6 8l2.5 2L6 12M10.5 12.5h3.5"
            }
            Self::Sliders => {
                "M3.5 6.5h7M15.5 6.5h1M11 6.5a2 2 0 1 0 4 0a2 2 0 1 0 -4 0zM3.5 13.5h1M9.5 13.5h7M5 13.5a2 2 0 1 0 4 0a2 2 0 1 0 -4 0z"
            }
            Self::Power => "M10 3v6M6.2 5.4a6.25 6.25 0 1 0 7.6 0",
            Self::Maximize => "M3.5 7.5v-4h4M12.5 3.5h4v4M16.5 12.5v4h-4M7.5 16.5h-4v-4",
            // A window with its toolbar and sidebar marked: "hide the
            // chrome". Deliberately unlike Maximize's four corners.
            Self::Focus => {
                "M5 3.5h10a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1 -2.5 2.5h-10a2.5 2.5 0 0 1 -2.5 -2.5v-8a2.5 2.5 0 0 1 2.5 -2.5zM2.5 7h15M6.5 7v9.5M10 10.5h4.5"
            }
            Self::Keyboard => {
                "M4.5 5h11a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-11a2 2 0 0 1 -2 -2v-6a2 2 0 0 1 2 -2zM6 8.5h.01M8.67 8.5h.01M11.33 8.5h.01M14 8.5h.01M7 12h6"
            }
            Self::Mouse => {
                "M10 2.5h0a4 4 0 0 1 4 4v7a4 4 0 0 1 -4 4h0a4 4 0 0 1 -4 -4v-7a4 4 0 0 1 4 -4zM10 5.5v3"
            }
            Self::Volume => {
                "M3.5 8v4h3l4 3.5v-11l-4 3.5zM13.5 7.5a3.5 3.5 0 0 1 0 5M15.5 5a7 7 0 0 1 0 10"
            }
            Self::VolumeOff => "M3.5 8v4h3l4 3.5v-11l-4 3.5zM13.5 8l4 4M17.5 8l-4 4",
            Self::ScanText => {
                "M3.5 6.5v-1a2 2 0 0 1 2-2h1M13.5 3.5h1a2 2 0 0 1 2 2v1M16.5 13.5v1a2 2 0 0 1-2 2h-1M6.5 16.5h-1a2 2 0 0 1-2-2v-1M7 8h6M7 10.5h6M7 13h3.5"
            }
            Self::Close => "M5.5 5.5l9 9M14.5 5.5l-9 9",
            Self::Plus => "M10 4.5v11M4.5 10h11",
            Self::Minus => "M4.5 10h11",
            Self::Sun => {
                "M7 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM10 2.5v1.5M10 16v1.5M2.5 10h1.5M16 10h1.5M4.7 4.7l1.06 1.06M14.24 14.24l1.06 1.06M4.7 15.3l1.06-1.06M14.24 5.76l1.06-1.06"
            }
            Self::Moon => "M16 12.3A6.5 6.5 0 0 1 7.7 4a6.5 6.5 0 1 0 8.3 8.3z",
            Self::Alert => "M10 3.5l7 12.5H3zM10 8.5v3M10 13.75h.01",
            Self::Languages => {
                "M3 5h8M7 3.5V5M9.5 5c-.6 3-2.6 5.6-5.5 7M5.5 8c.9 1.8 2.3 3.2 4 4M10.5 17l3-7 3 7M11.5 14.8h4"
            }
        }
    }
}

#[component]
pub fn Icon(name: IconName) -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 20 20" aria-hidden="true">
            <path d=name.path()></path>
        </svg>
    }
}
