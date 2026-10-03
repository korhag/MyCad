//! Basic AutoCAD linetype dash patterns.

pub fn normalize_linetype_name(name: &str) -> String {
    name.trim().to_ascii_uppercase()
}

pub fn is_bylayer_name(name: &str) -> bool {
    let n = normalize_linetype_name(name);
    n.is_empty() || n == "BYLAYER"
}

pub fn is_byblock_name(name: &str) -> bool {
    normalize_linetype_name(name) == "BYBLOCK"
}

pub fn is_continuous_name(name: &str) -> bool {
    matches!(
        normalize_linetype_name(name).as_str(),
        "CONTINUOUS" | "BYLAYER" | "BYBLOCK" | ""
    )
}

// ------------------------------------------------------------
// Type: LineTypeShape
// Purpose: One complex dash (shape or text) stored with the pattern.
//          The viewport draws dash lengths only.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct LineTypeShape {
    pub flag: u16,
    pub shapecode: u16,
    pub text: String,
    pub scale: f64,
    pub rotation: f64,
    pub x_offset: f64,
    pub y_offset: f64,
    pub style: String,
    /// Shape-file font, such as `ltypeshp.shx`, when the dash is not a text style.
    pub shape_file: String,
}

impl Default for LineTypeShape {
    fn default() -> Self {
        Self {
            flag: 0,
            shapecode: 0,
            text: String::new(),
            scale: 1.0,
            rotation: 0.0,
            x_offset: 0.0,
            y_offset: 0.0,
            style: String::new(),
            shape_file: String::new(),
        }
    }
}

// ------------------------------------------------------------
// Type: LineType
// Purpose: Named dash pattern in world units (positive = dash,
//          negative = gap, zero = dot). Empty pattern is continuous.
//          `shapes` is parallel to `dashes`; None is a plain dash.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct LineType {
    pub name: String,
    pub dashes: Vec<f64>,
    pub shapes: Vec<Option<LineTypeShape>>,
}

impl LineType {
    pub fn continuous(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dashes: Vec::new(),
            shapes: Vec::new(),
        }
    }

    pub fn is_continuous(&self) -> bool {
        self.dashes.is_empty() || self.dashes.iter().all(|d| *d >= 0.0 && d.abs() < 1e-15)
    }

    pub fn builtin(name: &str) -> Self {
        let upper = name.to_ascii_uppercase();
        let dashes = match upper.as_str() {
            "CONTINUOUS" | "BYLAYER" | "BYBLOCK" | "" => Vec::new(),
            "DASHED" | "DASHED2" | "DASHEDX2" => vec![12.0, -6.0],
            "HIDDEN" | "HIDDEN2" | "HIDDENX2" => vec![6.0, -3.0],
            "CENTER" | "CENTER2" | "CENTERX2" => vec![32.0, -6.0, 4.0, -6.0],
            "PHANTOM" | "PHANTOM2" | "PHANTOMX2" => vec![32.0, -6.0, 4.0, -6.0, 4.0, -6.0],
            "DOT" | "DOT2" | "DOTX2" => vec![0.0, -6.0],
            "DASHDOT" | "DASHDOT2" | "DASHDOTX2" => vec![12.0, -6.0, 0.0, -6.0],
            "DIVIDE" | "DIVIDE2" | "DIVIDEX2" => vec![12.0, -6.0, 0.0, -6.0, 0.0, -6.0],
            "BORDER" | "BORDER2" | "BORDERX2" => vec![12.0, -6.0, 12.0, -6.0, 0.0, -6.0],
            _ => Vec::new(),
        };
        Self {
            name: name.to_string(),
            dashes,
            shapes: Vec::new(),
        }
    }

    pub fn shape_at(&self, index: usize) -> Option<&LineTypeShape> {
        self.shapes.get(index).and_then(|shape| shape.as_ref())
    }
}
