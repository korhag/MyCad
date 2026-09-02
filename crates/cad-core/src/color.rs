//! AutoCAD Color Index (ACI) palette and ByLayer / ByBlock resolution.

// ------------------------------------------------------------
// Enum: CadColor
// Purpose: Entity or layer color before viewport RGB resolution.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadColor {
    ByLayer,
    ByBlock,
    Aci(u8),
    Rgb { r: u8, g: u8, b: u8 },
}

impl Default for CadColor {
    fn default() -> Self {
        Self::ByLayer
    }
}

impl CadColor {
    pub fn from_aci_index(index: i16) -> Self {
        match index {
            0 => Self::ByBlock,
            256 => Self::ByLayer,
            n if (1..=255).contains(&n) => Self::Aci(n as u8),
            _ => Self::ByLayer,
        }
    }

    pub fn resolve(self, layer: Self, block_inherit: Self) -> Rgb {
        match self {
            Self::ByLayer => layer.to_rgb_or_default(),
            Self::ByBlock => block_inherit.resolve(layer, CadColor::Aci(7)),
            Self::Aci(i) => aci_rgb(i),
            Self::Rgb { r, g, b } => Rgb { r, g, b },
        }
    }

    fn to_rgb_or_default(self) -> Rgb {
        match self {
            Self::Aci(i) => aci_rgb(i),
            Self::Rgb { r, g, b } => Rgb { r, g, b },
            Self::ByLayer | Self::ByBlock => aci_rgb(7),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn to_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            1.0,
        ]
    }
}

// Standard AutoCAD ACI palette (indices 1..=255). Index 0 is unused here.
const ACI: [[u8; 3]; 256] = generate_aci_palette();

const fn generate_aci_palette() -> [[u8; 3]; 256] {
    let mut pal = [[0u8; 3]; 256];
    pal[1] = [255, 0, 0];
    pal[2] = [255, 255, 0];
    pal[3] = [0, 255, 0];
    pal[4] = [0, 255, 255];
    pal[5] = [0, 0, 255];
    pal[6] = [255, 0, 255];
    pal[7] = [255, 255, 255];
    pal[8] = [128, 128, 128];
    pal[9] = [192, 192, 192];

    // Rows 10-249: 24 hues × 10 shades, matching AutoCAD's published table.
    let hues: [[u8; 3]; 9] = [
        [255, 0, 0],
        [255, 127, 0],
        [255, 255, 0],
        [127, 255, 0],
        [0, 255, 0],
        [0, 255, 127],
        [0, 255, 255],
        [0, 127, 255],
        [0, 0, 255],
    ];
    let more_hues: [[u8; 3]; 15] = [
        [127, 0, 255],
        [255, 0, 255],
        [255, 0, 127],
        [255, 63, 63],
        [255, 127, 63],
        [255, 255, 63],
        [127, 255, 63],
        [63, 255, 63],
        [63, 255, 127],
        [63, 255, 255],
        [63, 127, 255],
        [63, 63, 255],
        [127, 63, 255],
        [255, 63, 255],
        [255, 63, 127],
    ];
    let shades: [u8; 10] = [255, 191, 159, 127, 95, 63, 31, 223, 175, 111];
    let mut idx = 10;
    let mut h = 0;
    while h < 9 {
        let mut s = 0;
        while s < 10 {
            let factor = shades[s] as u32;
            pal[idx][0] = ((hues[h][0] as u32 * factor) / 255) as u8;
            pal[idx][1] = ((hues[h][1] as u32 * factor) / 255) as u8;
            pal[idx][2] = ((hues[h][2] as u32 * factor) / 255) as u8;
            idx += 1;
            s += 1;
        }
        h += 1;
    }
    let mut h = 0;
    while h < 15 && idx < 250 {
        let mut s = 0;
        while s < 10 && idx < 250 {
            let factor = shades[s] as u32;
            pal[idx][0] = ((more_hues[h][0] as u32 * factor) / 255) as u8;
            pal[idx][1] = ((more_hues[h][1] as u32 * factor) / 255) as u8;
            pal[idx][2] = ((more_hues[h][2] as u32 * factor) / 255) as u8;
            idx += 1;
            s += 1;
        }
        h += 1;
    }
    pal[250] = [51, 51, 51];
    pal[251] = [80, 80, 80];
    pal[252] = [105, 105, 105];
    pal[253] = [130, 130, 130];
    pal[254] = [190, 190, 190];
    pal[255] = [255, 255, 255];
    pal
}

pub fn aci_rgb(index: u8) -> Rgb {
    if index == 0 {
        return Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
    }
    let [r, g, b] = ACI[index as usize];
    Rgb { r, g, b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bylayer_uses_layer_aci() {
        let rgb = CadColor::ByLayer.resolve(CadColor::Aci(1), CadColor::Aci(7));
        assert_eq!(rgb, aci_rgb(1));
    }

    #[test]
    fn byblock_uses_insert_color() {
        let rgb = CadColor::ByBlock.resolve(CadColor::Aci(3), CadColor::Aci(5));
        assert_eq!(rgb, aci_rgb(5));
    }

    #[test]
    fn explicit_aci_ignores_layer() {
        let rgb = CadColor::Aci(4).resolve(CadColor::Aci(1), CadColor::Aci(5));
        assert_eq!(rgb, aci_rgb(4));
    }
}
