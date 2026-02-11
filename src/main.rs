use std::fs;
use lyon::path::Path;
use lyon::path::iterator::PathIterator;
use clipper2::*;

const TOLERANCE: f32 = 0.25;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("SVG Combiner - usvg + lyon + clipper2");
    println!("======================================\n");
    
    // Read and parse SVG
    let svg = fs::read("./init.svg")?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &opt)?;
    
    println!("✓ Parsed SVG with usvg");
    println!("  Size: {}x{}", tree.size().width(), tree.size().height());
    
    // ---------------- SVG → lyon paths ----------------
    let mut paths = Vec::<Path>::new();
    
    // Recursively extract paths from tree
    fn extract_paths(node: &usvg::Node, paths: &mut Vec<Path>) {
        match node {
            usvg::Node::Path(path) => {
                let mut builder = Path::builder();
                
                for segment in path.data().segments() {
                    match segment {
                        usvg::tiny_skia_path::PathSegment::MoveTo(p) => {
                            builder.begin((p.x, p.y).into());
                        }
                        usvg::tiny_skia_path::PathSegment::LineTo(p) => {
                            builder.line_to((p.x, p.y).into());
                        }
                        usvg::tiny_skia_path::PathSegment::QuadTo(p1, p2) => {
                            builder.quadratic_bezier_to(
                                (p1.x, p1.y).into(),
                                (p2.x, p2.y).into(),
                            );
                        }
                        usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p3) => {
                            builder.cubic_bezier_to(
                                (p1.x, p1.y).into(),
                                (p2.x, p2.y).into(),
                                (p3.x, p3.y).into(),
                            );
                        }
                        usvg::tiny_skia_path::PathSegment::Close => {
                            builder.close();
                        }
                    }
                }
                
                paths.push(builder.build());
            }
            usvg::Node::Group(group) => {
                for child in group.children() {
                    extract_paths(child, paths);
                }
            }
            _ => {}
        }
    }
   
   for node in tree.root().children() {
    extract_paths(node, &mut paths);
  }
    
    println!("\n✓ Extracted {} path(s)", paths.len());
    
    // ---------------- Flatten → Clipper polygons ----------------
    // clipper2 0.4 uses Point<P> with a PointScaler type parameter
    // Default is Centi (centipixels), so we use Point::new(x, y)
    let mut subject: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut subject_groups: Vec<Vec<Vec<(f64, f64)>>> = Vec::new();
    
    for (idx, path) in paths.iter().enumerate() {
        let mut current_polygon = Vec::new();
        
        use lyon::path::Event::*;
        for event in path.iter().flattened(TOLERANCE) {
            match event {
                Begin { at } => {
                    current_polygon = Vec::new();
                    current_polygon.push((
                        at.x as f64,
                        at.y as f64,
                    ));
                }
                Line { to, .. } => {
                    current_polygon.push((
                        to.x as f64,
                        to.y as f64,
                    ));
                }
                End { close, .. } => {
                    if close && current_polygon.len() >= 3 {
                        subject.push(current_polygon.clone());
                    }
                }
                _ => {}
            }
        }
        if !subject.is_empty() {
            subject_groups.push(subject.clone().into());
        }
        println!("  Path {}: {} polygon(s)", idx + 1, &subject.len());
        subject = Vec::new();
    }
    println!("\n✓ Created {} path group(s)", &subject_groups.len()); 
    
    // ---------------- Union with Clipper2 ----------------
    println!("\n🔄 Computing inflate with Clipper2...");
    
let mut solution: Paths<Centi> = Paths::new(vec![]);
let empty: Paths<Centi> = Paths::new(vec![]);

for g in &subject_groups {
  let changed = inflate(g.clone(), -1.0, JoinType::Round, EndType::Polygon, 0.0)
    .simplify(0.2, false);

  //let changed = difference(g.clone(), empty.clone(), FillRule::NonZero);

  for poly in changed.iter() {
    solution.push(poly.clone());
  }
}

let empty: Paths<Centi> = Paths::new(vec![]);
let combined = if solution.len() > 1 {
  let s: Vec<_> = solution.iter().cloned().collect();
  union::<Centi>(s[1..].to_vec(), vec![s[0].clone()], FillRule::NonZero).unwrap()
} else {
  solution.clone()
};

    println!("✓ Union complete: {} polygon(s) in result", solution.len());
// After the inflate/difference loop, group by original shape
// Simpler: combine all resulting polygons into one multi-subpath
let mut d = String::new();
for poly in combined.iter() {
    if poly.is_empty() {
        continue;
    }
    
    let points: Vec<_> = poly.iter().collect();
    if points.is_empty() {
        continue;
    }
    
    let first = points[0];
    d.push_str(&format!("M {:.4} {:.4} ", first.x(), first.y()));
    for pt in points.iter().skip(1) {
        d.push_str(&format!("L {:.4} {:.4} ", pt.x(), pt.y()));
    }
    d.push_str("Z ");
}

let output_svg = format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" 
     viewBox="{} {} {} {}" 
     width="{}mm" 
     height="{}mm">
    <path d="{}" fill="black" fill-rule="nonzero" stroke="none"/>
</svg>"#,
    0.0, 0.0,
    tree.size().width(), tree.size().height(),
    tree.size().width(), tree.size().height(),
    d.trim()
);    
    fs::write("./output.svg", &output_svg)?;
    
    println!("\n✅ Success!");
    println!("Output saved to: output.svg");
    
    // Statistics
    let input_vertices: usize = subject.iter().map(|p| p.len()).sum();
    let output_vertices: usize = solution.iter().map(|p| p.len()).sum();
    
    println!("\n📊 Statistics:");
    println!("  Input polygons: {}", subject.len());
    println!("  Input vertices: {}", input_vertices);
    println!("  Output polygons: {}", solution.len());
    println!("  Output vertices: {}", output_vertices);
    
    if output_vertices < input_vertices {
        let reduction = 100.0 * (1.0 - output_vertices as f64 / input_vertices as f64);
        println!("  Vertex reduction: {:.1}%", reduction);
    }

    Ok(())
}
