//! Headless raster preview of tessellated linework for visual regression.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use cad_core::Extents2;
use cad_render::DisplayList;

pub fn write_preview_ppm(
    path: impl AsRef<Path>,
    display: &DisplayList,
    extents: Extents2,
    width: u32,
    height: u32,
) -> std::io::Result<()> {
    let extents = extents.padded(0.04).expanded_to_square_if_degenerate();
    let width = width.max(2);
    let height = height.max(2);
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    let world_w = extents.width().max(1e-9);
    let world_h = extents.height().max(1e-9);
    let origin = display.origin;

    let to_pixel = |wx: f64, wy: f64| -> (i32, i32) {
        let px = ((wx - extents.min.x) / world_w) * (width as f64 - 1.0);
        let py = (1.0 - (wy - extents.min.y) / world_h) * (height as f64 - 1.0);
        (px.round() as i32, py.round() as i32)
    };

    let verts = &display.line_vertices;
    let mut i = 0;
    while i + 1 < verts.len() {
        let a = verts[i];
        let b = verts[i + 1];
        i += 2;
        let ax = origin.x + a.position[0] as f64;
        let ay = origin.y + a.position[1] as f64;
        let bx = origin.x + b.position[0] as f64;
        let by = origin.y + b.position[1] as f64;
        let (x0, y0) = to_pixel(ax, ay);
        let (x1, y1) = to_pixel(bx, by);
        let r = (a.color[0].clamp(0.0, 1.0) * 255.0) as u8;
        let g = (a.color[1].clamp(0.0, 1.0) * 255.0) as u8;
        let bl = (a.color[2].clamp(0.0, 1.0) * 255.0) as u8;
        draw_line(&mut pixels, width, height, x0, y0, x1, y1, r, g, bl);
    }

    let file = File::create(path)?;
    let mut out = BufWriter::new(file);
    writeln!(out, "P6\n{width} {height}\n255")?;
    out.write_all(&pixels)?;
    Ok(())
}

fn draw_line(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    r: u8,
    g: u8,
    b: u8,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put_pixel(pixels, width, height, x0, y0, r, g, b);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
        if x0 < -8 || y0 < -8 || x0 > width as i32 + 8 || y0 > height as i32 + 8 {
            if (x0 < 0 && x1 < 0)
                || (y0 < 0 && y1 < 0)
                || (x0 >= width as i32 && x1 >= width as i32)
                || (y0 >= height as i32 && y1 >= height as i32)
            {
                break;
            }
        }
    }
}

fn put_pixel(pixels: &mut [u8], width: u32, height: u32, x: i32, y: i32, r: u8, g: u8, b: u8) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as u32 * width + x as u32) * 3) as usize;
    pixels[idx] = r;
    pixels[idx + 1] = g;
    pixels[idx + 2] = b;
}
