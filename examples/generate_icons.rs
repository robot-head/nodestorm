use image::ImageFormat;
use nodestorm::icon;
use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf};

fn mark_elements(color: &str) -> String {
    let circles = icon::NODE_INDICES
        .iter()
        .map(|&index| {
            let (x, y) = icon::BOLT_POINTS[index];
            format!(
                "  <circle cx=\"{x}\" cy=\"{y}\" r=\"{}\" fill=\"{color}\"/>\n",
                icon::NODE_RADIUS
            )
        })
        .collect::<String>();
    format!(
        "  <polyline points=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n{circles}",
        icon::svg_points(),
        icon::STROKE_WIDTH,
    )
}

fn mark_svg() -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{}\">\n{}</svg>\n",
        icon::VIEW_BOX,
        mark_elements("currentColor"),
    )
}

fn tile_svg() -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{}\">\n  <rect x=\"8\" y=\"8\" width=\"240\" height=\"240\" rx=\"42\" fill=\"#24272D\"/>\n{}</svg>\n",
        icon::VIEW_BOX,
        mark_elements("white"),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let check = std::env::args().skip(1).any(|arg| arg == "--check");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    let mut outputs = BTreeMap::new();
    outputs.insert("nodestorm-mark.svg".into(), mark_svg().into_bytes());
    outputs.insert("nodestorm-tile.svg".into(), tile_svg().into_bytes());
    for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(icon::render_tile(size))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
        outputs.insert(format!("nodestorm-{size}.png"), bytes);
    }

    if check {
        for (name, expected) in &outputs {
            let actual = fs::read(root.join(name))?;
            assert_eq!(&actual, expected, "stale generated asset: {name}");
        }
    } else {
        fs::create_dir_all(&root)?;
        for (name, bytes) in outputs {
            fs::write(root.join(name), bytes)?;
        }
    }

    Ok(())
}
