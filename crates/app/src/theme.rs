//! The Solarized palette, in both its light and dark forms.
//!
//! Solarized keeps the same eight accent colours across both modes and only
//! swaps the greys, which is exactly what a node graph wants: wire colours stay
//! meaningful when you flip the theme.

use cat_paws_core::{Category, DataType};
use egui::Color32;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// The greys, darkest to lightest.
const BASE03: Color32 = rgb(0x002b36);
const BASE02: Color32 = rgb(0x073642);
const BASE01: Color32 = rgb(0x586e75);
const BASE00: Color32 = rgb(0x657b83);
const BASE0: Color32 = rgb(0x839496);
const BASE1: Color32 = rgb(0x93a1a1);
const BASE2: Color32 = rgb(0xeee8d5);
const BASE3: Color32 = rgb(0xfdf6e3);

// The accents, identical in both themes.
const YELLOW: Color32 = rgb(0xb58900);
const ORANGE: Color32 = rgb(0xcb4b16);
const RED: Color32 = rgb(0xdc322f);
const MAGENTA: Color32 = rgb(0xd33682);
const VIOLET: Color32 = rgb(0x6c71c4);
const BLUE: Color32 = rgb(0x268bd2);
const CYAN: Color32 = rgb(0x2aa198);
const GREEN: Color32 = rgb(0x859900);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub mode: Mode,
    pub canvas: Color32,
    pub panel: Color32,
    pub node_body: Color32,
    pub node_outline: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_faint: Color32,
    pub grid: Color32,
    pub grid_strong: Color32,
    pub exec_wire: Color32,
    pub selection: Color32,
    pub error: Color32,
    pub warning: Color32,
}

impl Palette {
    pub fn new(mode: Mode) -> Palette {
        match mode {
            Mode::Dark => Palette {
                mode,
                canvas: BASE03,
                panel: BASE02,
                node_body: rgb(0x0a3b47),
                node_outline: BASE01,
                text: BASE0,
                text_strong: BASE1,
                text_faint: BASE01,
                grid: rgb(0x00343f),
                grid_strong: rgb(0x0a3b47),
                exec_wire: BASE1,
                selection: YELLOW,
                error: RED,
                warning: ORANGE,
            },
            Mode::Light => Palette {
                mode,
                canvas: BASE3,
                panel: BASE2,
                node_body: rgb(0xf5efdc),
                node_outline: BASE1,
                text: BASE00,
                text_strong: BASE01,
                text_faint: BASE1,
                grid: rgb(0xf7f1de),
                grid_strong: BASE2,
                exec_wire: BASE01,
                selection: ORANGE,
                error: RED,
                warning: ORANGE,
            },
        }
    }

    /// Wire and pin colour for a data type. These are the colours the reference
    /// image calls out: cyan integer, red boolean, pink string, green float.
    pub fn data_color(&self, ty: DataType) -> Color32 {
        match ty {
            DataType::Int => CYAN,
            DataType::Bool => RED,
            DataType::Str => MAGENTA,
            DataType::Float => GREEN,
        }
    }

    /// Header colour for a node, by category.
    pub fn category_color(&self, category: Category) -> Color32 {
        match category {
            Category::Event => GREEN,
            Category::Flow => match self.mode {
                Mode::Dark => BASE01,
                Mode::Light => BASE00,
            },
            Category::Action => VIOLET,
            Category::Pure => BLUE,
            Category::Variable => CYAN,
        }
    }

    /// Text drawn on top of a category header.
    pub fn on_category(&self) -> Color32 {
        Color32::from_rgb(0xfd, 0xf6, 0xe3)
    }

    /// Applies the matching egui widget styling, so panels and buttons agree
    /// with the canvas instead of using egui's stock greys.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = match self.mode {
            Mode::Dark => egui::Visuals::dark(),
            Mode::Light => egui::Visuals::light(),
        };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = self.canvas;
        visuals.faint_bg_color = self.node_body;
        visuals.override_text_color = Some(self.text);
        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.inactive.bg_fill = self.node_body;
        visuals.widgets.hovered.bg_fill = self.grid_strong;
        visuals.widgets.active.bg_fill = self.grid_strong;
        visuals.selection.bg_fill = self.category_color(Category::Pure).gamma_multiply(0.5);
        ctx.set_visuals(visuals);
    }
}
