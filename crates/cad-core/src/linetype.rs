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
// Type: LineType
// Purpose: Named dash pattern in world units (positive = dash,
//          negative = gap, zero = dot). Empty pattern is continuous.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct LineType {
    pub name: String,
    pub dashes: Vec<f64>,
}

impl LineType {
    pub fn continuous(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dashes: Vec::new(),
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
        }
    }
}
