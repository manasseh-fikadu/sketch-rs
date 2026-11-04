//! Hand-drawn style settings (the geometry lives in `crate::render::rough`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillStyle {
    #[default]
    Hachure,
    CrossHatch,
    Solid,
    Zigzag,
    Dots,
}

impl FillStyle {
    pub const ALL: [FillStyle; 5] = [
        FillStyle::Hachure,
        FillStyle::CrossHatch,
        FillStyle::Solid,
        FillStyle::Zigzag,
        FillStyle::Dots,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FillStyle::Hachure => "Hachure",
            FillStyle::CrossHatch => "Cross-hatch",
            FillStyle::Solid => "Solid",
            FillStyle::Zigzag => "Zigzag",
            FillStyle::Dots => "Dots",
        }
    }
}

/// Per-node sloppiness, as in Excalidraw (`roughness` 0 = architect, 1 = artist, 2 = cartoonist).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rough {
    pub roughness: f64,
    pub bowing: f64,
    /// Random seed so the wobble is stable across renders.
    pub seed: u32,
    pub fill_style: FillStyle,
    /// Hachure line spacing; defaults to 4 × stroke width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hachure_gap: Option<f64>,
    pub hachure_angle: f64,
}

impl Default for Rough {
    fn default() -> Self {
        Rough {
            roughness: 1.0,
            bowing: 1.0,
            seed: 1,
            fill_style: FillStyle::Hachure,
            hachure_gap: None,
            hachure_angle: -41.0,
        }
    }
}

impl Rough {
    pub fn with_seed(seed: u32) -> Rough {
        Rough {
            seed: seed.max(1),
            ..Rough::default()
        }
    }
}
