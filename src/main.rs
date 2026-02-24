use std::fs;
use lyon::path::Path;
use lyon::path::iterator::PathIterator;
use clipper2::*;
use usvg::tiny_skia_path::PathSegment;

const TOLERANCE: f32 = 0.1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read and parse SVG
    let svg = fs::read("./init.svg")?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &opt)?;
    
    println!("✓ Parsed SVG with usvg");
    println!("  Size: {}px x {}px", tree.size().width(), tree.size().height());
    
    // ---------------- SVG → lyon paths ----------------
    let mut paths = Vec::<Path>::new();
    
    // Recursively extract paths from tree
    fn extract_paths(node: &usvg::Node, paths: &mut Vec<Path>) {
        match node {
            usvg::Node::Path(path) => {
                // Lyon builder
                let mut builder = Path::builder();
                // May cycles over many subpaths
                for segment in path.data().segments() {
                    match segment {
                        PathSegment::MoveTo(p) => {
                            builder.begin((p.x, p.y).into());
                        }
                        PathSegment::LineTo(p) => {
                            builder.line_to((p.x, p.y).into());
                        }
                        PathSegment::QuadTo(p1, p2) => {
                            builder.quadratic_bezier_to(
                                (p1.x, p1.y).into(),
                                (p2.x, p2.y).into(),
                            );
                        }
                        PathSegment::CubicTo(p1, p2, p3) => {
                            builder.cubic_bezier_to(
                                (p1.x, p1.y).into(),
                                (p2.x, p2.y).into(),
                                (p3.x, p3.y).into(),
                            );
                        }
                        PathSegment::Close => {
                            builder.close();
                        }
                    }
                }
                // exhausted navigating over the path with posible subpaths
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
  
    // start extracting paths
    for node in tree.root().children() {
      extract_paths(node, &mut paths);
    }
    
    println!("\n✓ Extracted {} path(s) that may have subpaths", paths.len());
    
    // ---------------- Flatten → Clipper polygons ----------------
    let mut contour_segments: Vec<Vec<(f64, f64)>> = Vec::new(); // by flattening a path is stored as a set of small segments (paths)
    let mut contour_segments_paths: Vec<Vec<Vec<(f64, f64)>>> = Vec::new(); // a flattened path with all its flattened subpaths
    let mut input_vertices: usize = 0;
    
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
                        contour_segments.push(current_polygon.clone());
                    }
                }
                _ => {}
            }
        }
        // exhausted flattened (segmented) path with all subpaths 
        if !contour_segments.is_empty() {
            contour_segments_paths.push(contour_segments.clone().into());
        }
        println!("  Path {}: {} polygon(s)", idx + 1, &contour_segments.len());
        input_vertices += contour_segments.iter().map(|p| p.len()).sum::<usize>();
        contour_segments = Vec::new()
    }
    println!("\n✓ Created {} path group(s)", &contour_segments_paths.len()); 
    
    println!("\n🔄 Computing inflate with Clipper2...");
    
    let  mut combined =  Paths::new(vec![]);
    for g in &contour_segments_paths {
      let expanded: Paths<Centi> = inflate(g.clone(), 10.0, JoinType::Round, EndType::Polygon, 0.0);

      combined = if combined.is_empty() {
        g.clone().into()
      } else {
        combined = difference(combined, expanded, FillRule::NonZero)?;
        union(combined, g.clone(), FillRule::NonZero)?
      };
    }

    // Cleaning
    combined = combined.simplify(0.2, true);
    combined = filter_small(combined, 50.0);
    combined = union(combined, Paths::new(vec![]), FillRule::NonZero)?;
    combined = combined.simplify(0.1, true);

    let output_polygons = combined.iter().collect::<Vec<_>>().len();
    println!("✓ Union complete: {} polygon(s) in result", output_polygons);
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
        d.push_str(&format!("M {} {} ", first.x(), first.y()));
        for pt in points.iter().skip(1) {
            d.push_str(&format!("L {} {} ", pt.x(), pt.y()));
        }
        d.push_str("Z ");
    }

    let output_svg = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <svg xmlns="http://www.w3.org/2000/svg" 
         viewBox="{} {} {} {}" 
         width="{}px" 
         height="{}px">
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
        let output_vertices: usize = combined.iter().map(|p| p.len()).sum();
        
        println!("\n📊 Statistics:");
        println!("  Input vertices: {}", input_vertices);
        println!("  Output polygons: {}", output_polygons);
        println!("  Output vertices: {}", output_vertices);
        
        if output_vertices < input_vertices {
            let reduction = 100.0 * (1.0 - output_vertices as f64 / input_vertices as f64);
            println!("  Vertex reduction: {:.1}%", reduction);
        }

        Ok(())
    }

    fn filter_small(paths: Paths<Centi>, min_area: f64) -> Paths<Centi> {
        Paths::new(
            paths
                .into_iter()
                .filter(|p| p.signed_area().abs() >= min_area)
                .collect(),
        )
    }
