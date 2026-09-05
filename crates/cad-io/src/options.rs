//! Options for native DXF and PDF export.

use cad_core::Extents2;

use std::path::Path;

// ------------------------------------------------------------
// Enum: CadFileFormat
// Purpose: On-disk CAD identity for save/export. DWG conversion
//          still lives in dwg-import because of LibreDWG.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadFileFormat {
    Dxf,
    Dwg,
}

impl CadFileFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Dxf => "dxf",
            Self::Dwg => "dwg",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| {
                if ext.eq_ignore_ascii_case("dxf") {
                    Some(Self::Dxf)
                } else if ext.eq_ignore_ascii_case("dwg") {
                    Some(Self::Dwg)
                } else {
                    None
                }
            })
    }
}

// ------------------------------------------------------------
// Enum: DxfAcadVersion
// Purpose: $ACADVER written by the native ASCII DXF writer.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxfAcadVersion {
    /// AutoCAD 2000 (AC1015). LibreDWG can convert this to DWG.
    R2000,
}

impl DxfAcadVersion {
    pub fn acadver(self) -> &'static str {
        match self {
            Self::R2000 => "AC1015",
        }
    }
}

impl Default for DxfAcadVersion {
    fn default() -> Self {
        Self::R2000
    }
}

// ------------------------------------------------------------
// Type: DxfExportOptions
// Purpose: Native DXF writer settings. Binary DXF is not offered.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxfExportOptions {
    pub version: DxfAcadVersion,
}

impl Default for DxfExportOptions {
    fn default() -> Self {
        Self {
            version: DxfAcadVersion::R2000,
        }
    }
}

// ------------------------------------------------------------
// Enum: PdfPaperSize
// Purpose: ISO A-series sheets offered in the plot dialog.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfPaperSize {
    A4,
    A3,
    A2,
    A1,
    A0,
}

impl PdfPaperSize {
    pub const ALL: [Self; 5] = [Self::A4, Self::A3, Self::A2, Self::A1, Self::A0];

    pub fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::A2 => "A2",
            Self::A1 => "A1",
            Self::A0 => "A0",
        }
    }

    /// Portrait width × height in millimetres (ISO 216).
    pub fn portrait_mm(self) -> (f64, f64) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::A3 => (297.0, 420.0),
            Self::A2 => (420.0, 594.0),
            Self::A1 => (594.0, 841.0),
            Self::A0 => (841.0, 1189.0),
        }
    }
}

impl Default for PdfPaperSize {
    fn default() -> Self {
        Self::A4
    }
}

// ------------------------------------------------------------
// Enum: PdfOrientation
// Purpose: Paper rotation for the plot.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfOrientation {
    Portrait,
    Landscape,
}

impl Default for PdfOrientation {
    fn default() -> Self {
        Self::Portrait
    }
}

// ------------------------------------------------------------
// Enum: PdfPlotStyle
// Purpose: Stroke colors on the printed page.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfPlotStyle {
    Color,
    Monochrome,
}

impl Default for PdfPlotStyle {
    fn default() -> Self {
        Self::Color
    }
}

pub const PDF_MARGIN_MM: [f64; 3] = [5.0, 10.0, 15.0];

pub const PDF_STROKE_THIN_PT: f64 = 0.25;
pub const PDF_STROKE_NORMAL_PT: f64 = 0.5;
pub const PDF_STROKE_HEAVY_PT: f64 = 1.0;

pub const PDF_STROKE_WEIGHTS: [(f64, &'static str); 3] = [
    (PDF_STROKE_THIN_PT, "Thin"),
    (PDF_STROKE_NORMAL_PT, "Normal"),
    (PDF_STROKE_HEAVY_PT, "Heavy"),
];

pub fn mm_to_pt(mm: f64) -> f64 {
    mm * 72.0 / 25.4
}

// ------------------------------------------------------------
// Enum: PdfPlotArea
// Purpose: World region mapped onto the sheet. Window is a clip,
//          not an entity-intersection filter.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PdfPlotArea {
    Extents,
    Window(Extents2),
}

impl Default for PdfPlotArea {
    fn default() -> Self {
        Self::Extents
    }
}

impl PdfPlotArea {
    pub fn is_window(self) -> bool {
        matches!(self, Self::Window(_))
    }

    pub fn window(self) -> Option<Extents2> {
        match self {
            Self::Extents => None,
            Self::Window(extents) => Some(extents),
        }
    }
}

// ------------------------------------------------------------
// Type: PdfExportOptions
// Purpose: Vector plot of model space. Scale is fit-to-page.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfExportOptions {
    pub paper: PdfPaperSize,
    pub orientation: PdfOrientation,
    pub style: PdfPlotStyle,
    pub margin_mm: f64,
    pub stroke_pt: f64,
    pub plot_area: PdfPlotArea,
    pub center_plot: bool,
}

impl Default for PdfExportOptions {
    fn default() -> Self {
        Self {
            paper: PdfPaperSize::A4,
            orientation: PdfOrientation::Portrait,
            style: PdfPlotStyle::Color,
            margin_mm: 10.0,
            stroke_pt: PDF_STROKE_NORMAL_PT,
            plot_area: PdfPlotArea::Extents,
            center_plot: true,
        }
    }
}

impl PdfExportOptions {
    pub fn page_size_pt(self) -> (f64, f64) {
        let (width_mm, height_mm) = self.paper.portrait_mm();
        let (width_mm, height_mm) = match self.orientation {
            PdfOrientation::Portrait => (width_mm, height_mm),
            PdfOrientation::Landscape => (height_mm, width_mm),
        };
        (mm_to_pt(width_mm), mm_to_pt(height_mm))
    }

    pub fn margin_pt(self) -> f64 {
        mm_to_pt(self.margin_mm)
    }
}

// ------------------------------------------------------------
// Type: SaveReport
// Purpose: What the writer actually put on disk.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveReport {
    pub warnings: Vec<String>,
    pub entities_written: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cad_file_format_extensions() {
        assert_eq!(CadFileFormat::Dxf.extension(), "dxf");
        assert_eq!(CadFileFormat::Dwg.extension(), "dwg");
        assert_eq!(
            CadFileFormat::from_path(Path::new("plant.DWG")),
            Some(CadFileFormat::Dwg)
        );
        assert_eq!(
            CadFileFormat::from_path(Path::new("plant.dxf")),
            Some(CadFileFormat::Dxf)
        );
        assert_eq!(CadFileFormat::from_path(Path::new("plant.pdf")), None);
    }

    #[test]
    fn a4_portrait_and_landscape_use_iso_millimetres() {
        let portrait = PdfExportOptions::default();
        let (width, height) = portrait.page_size_pt();
        assert!((width - mm_to_pt(210.0)).abs() < 1e-9);
        assert!((height - mm_to_pt(297.0)).abs() < 1e-9);
        assert!((portrait.margin_pt() - mm_to_pt(10.0)).abs() < 1e-9);
        let mut landscape = portrait;
        landscape.orientation = PdfOrientation::Landscape;
        let (width, height) = landscape.page_size_pt();
        assert!((width - mm_to_pt(297.0)).abs() < 1e-9);
        assert!((height - mm_to_pt(210.0)).abs() < 1e-9);
    }

    #[test]
    fn plot_dialog_choices_are_iso_paper_and_millimetre_margins() {
        assert_eq!(
            PdfPaperSize::ALL.map(PdfPaperSize::label),
            ["A4", "A3", "A2", "A1", "A0"]
        );
        assert_eq!(PDF_MARGIN_MM, [5.0, 10.0, 15.0]);
        assert_eq!(PdfPlotStyle::default(), PdfPlotStyle::Color);
        assert_eq!(PdfOrientation::default(), PdfOrientation::Portrait);
        assert_eq!(PdfPlotArea::default(), PdfPlotArea::Extents);
        assert!(PdfExportOptions::default().center_plot);
        assert_eq!(
            PDF_STROKE_WEIGHTS.map(|(_, label)| label),
            ["Thin", "Normal", "Heavy"]
        );
    }
}
