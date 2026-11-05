//! SWF shape geometry lowered into the existing Chaptera PathGeometryV1 vocabulary.
//! This is a typed, source-neutral adapter; rendering stays outside this crate.

use crate::PortError;
use crate::scene::{Color, Point, Scene, Shape, contours_for_fill};

pub const PATH_SCHEMA: &str = "chaptera.path-geometry.v1";
pub const EMU_PER_TWIP: i64 = 635;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathCommand {
    MoveTo { x: i64, y: i64 },
    LineTo { x: i64, y: i64 },
    QuadTo { cx: i64, cy: i64, x: i64, y: i64 },
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathGeometryV1 {
    pub schema: &'static str,
    pub commands: Vec<PathCommand>,
    pub fill_rule: &'static str,
    pub clip_rule: &'static str,
    pub bounds: BoundsEmu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundsEmu {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeFillPath {
    pub character_id: u16,
    pub fill_style_index: u16,
    pub color: Color,
    pub geometry: PathGeometryV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramePathInstance {
    pub character_id: u16,
    pub depth: u16,
    pub fill_style_index: u16,
    /// Translation in EMU. Linear matrix elements remain dimensionless.
    pub tx_emu: i64,
    pub ty_emu: i64,
}

fn emu(v: i32) -> i64 {
    i64::from(v) * EMU_PER_TWIP
}

fn extend_points(command: &PathCommand, points: &mut Vec<(i64, i64)>) {
    match *command {
        PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => points.push((x, y)),
        PathCommand::QuadTo { cx, cy, x, y } => {
            points.push((cx, cy));
            points.push((x, y));
        }
        PathCommand::Close => {}
    }
}

fn normalized_path(commands: Vec<PathCommand>) -> Result<PathGeometryV1, PortError> {
    let mut points = Vec::new();
    for cmd in &commands {
        extend_points(cmd, &mut points);
    }
    let first = *points.first().ok_or(PortError::Malformed("empty path"))?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.0, first.0, first.1, first.1);
    for (x, y) in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    Ok(PathGeometryV1 {
        schema: PATH_SCHEMA,
        commands,
        fill_rule: "nonzero",
        clip_rule: "nonzero",
        bounds: BoundsEmu {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        },
    })
}

/// Emits one normalized path per solid fill style, preserving separate contours
/// within a path. SWF line styles are refused until the shared stroke contract
/// is implemented, rather than being silently dropped.
pub fn shape_fill_paths(shape: &Shape) -> Result<Vec<ShapeFillPath>, PortError> {
    if !shape.lines.is_empty() {
        return Err(PortError::Unsupported("Chaptera stroke adapter pending"));
    }
    let mut paths = Vec::new();
    for (fill_index, color) in shape.fills.iter().enumerate() {
        let index = u16::try_from(fill_index + 1)
            .map_err(|_| PortError::Malformed("fill index overflow"))?;
        let contours = contours_for_fill(shape, index)?;
        if contours.is_empty() {
            continue;
        }
        let mut commands = Vec::new();
        for contour in contours {
            let Point { x, y } = contour[0].from;
            commands.push(PathCommand::MoveTo {
                x: emu(x),
                y: emu(y),
            });
            for edge in contour {
                if let Some(Point { x: cx, y: cy }) = edge.control {
                    commands.push(PathCommand::QuadTo {
                        cx: emu(cx),
                        cy: emu(cy),
                        x: emu(edge.to.x),
                        y: emu(edge.to.y),
                    });
                } else {
                    commands.push(PathCommand::LineTo {
                        x: emu(edge.to.x),
                        y: emu(edge.to.y),
                    });
                }
            }
            commands.push(PathCommand::Close);
        }
        paths.push(ShapeFillPath {
            character_id: shape.id,
            fill_style_index: index,
            color: *color,
            geometry: normalized_path(commands)?,
        });
    }
    Ok(paths)
}

pub fn scene_paths(scene: &Scene) -> Result<Vec<ShapeFillPath>, PortError> {
    let mut result = Vec::new();
    for shape in scene.shapes.values() {
        result.extend(shape_fill_paths(shape)?);
    }
    Ok(result)
}

/// A projection of an individual SWF frame. Depth is paint order, and
/// CharacterId identifies reusable geometry, not a unique placed instance.
pub fn frame_path_instances(
    scene: &Scene,
    frame_index: usize,
) -> Result<Vec<FramePathInstance>, PortError> {
    let frame = scene
        .frames
        .get(frame_index)
        .ok_or(PortError::Malformed("frame out of range"))?;
    let mut out = Vec::new();
    for instance in &frame.instances {
        let shape = scene
            .shapes
            .get(&instance.character_id)
            .ok_or(PortError::Malformed("missing placed character"))?;
        for path in shape_fill_paths(shape)? {
            out.push(FramePathInstance {
                character_id: instance.character_id,
                depth: instance.depth,
                fill_style_index: path.fill_style_index,
                tx_emu: emu(instance.matrix.tx),
                ty_emu: emu(instance.matrix.ty),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Edge, Frame, Instance, LineStyle, Matrix, Point, Rect};
    use std::collections::BTreeMap;

    fn rectangle() -> Shape {
        let points = [
            Point { x: 0, y: 0 },
            Point { x: 400, y: 0 },
            Point { x: 400, y: 200 },
            Point { x: 0, y: 200 },
            Point { x: 0, y: 0 },
        ];
        Shape {
            id: 7,
            bounds: Rect {
                xmin: 0,
                xmax: 400,
                ymin: 0,
                ymax: 200,
            },
            fills: vec![Color { r: 255, g: 0, b: 0 }],
            lines: vec![],
            edges: points
                .windows(2)
                .map(|p| Edge {
                    from: p[0],
                    control: None,
                    to: p[1],
                    fill_left: 0,
                    fill_right: 1,
                    line: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn chaptera_commands_and_emu_match_reference() {
        let paths = shape_fill_paths(&rectangle()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].geometry.schema, PATH_SCHEMA);
        assert_eq!(paths[0].geometry.fill_rule, "nonzero");
        assert_eq!(
            paths[0].geometry.bounds,
            BoundsEmu {
                x: 0,
                y: 0,
                width: 254000,
                height: 127000
            }
        );
        assert_eq!(
            paths[0].geometry.commands,
            vec![
                PathCommand::MoveTo { x: 0, y: 0 },
                PathCommand::LineTo { x: 254000, y: 0 },
                PathCommand::LineTo {
                    x: 254000,
                    y: 127000
                },
                PathCommand::LineTo { x: 0, y: 127000 },
                PathCommand::LineTo { x: 0, y: 0 },
                PathCommand::Close,
            ]
        );
    }

    #[test]
    fn three_frames_reuse_one_shape_but_move_instance() {
        let shape = rectangle();
        let first = Instance {
            character_id: 7,
            depth: 1,
            matrix: Matrix {
                tx: 600,
                ty: 400,
                ..Matrix::IDENTITY
            },
        };
        let second = Instance {
            character_id: 7,
            depth: 1,
            matrix: Matrix {
                tx: 1000,
                ty: 800,
                ..Matrix::IDENTITY
            },
        };
        let mut shapes = BTreeMap::new();
        shapes.insert(7, shape);
        let frame = |instances| Frame {
            background: Color {
                r: 230,
                g: 230,
                b: 230,
            },
            instances,
        };
        let scene = Scene {
            stage: Rect {
                xmin: 0,
                xmax: 3000,
                ymin: 0,
                ymax: 2000,
            },
            frame_rate: 24.0,
            shapes,
            frames: vec![frame(vec![first]), frame(vec![second]), frame(vec![])],
        };
        assert_eq!(scene_paths(&scene).unwrap().len(), 1);
        assert_eq!(frame_path_instances(&scene, 0).unwrap()[0].tx_emu, 381000);
        assert_eq!(frame_path_instances(&scene, 1).unwrap()[0].tx_emu, 635000);
        assert!(frame_path_instances(&scene, 2).unwrap().is_empty());
    }

    #[test]
    fn unsupported_strokes_fail_closed() {
        let mut shape = rectangle();
        shape.lines.push(LineStyle {
            width: 10,
            color: Color { r: 0, g: 0, b: 0 },
        });
        assert!(matches!(
            shape_fill_paths(&shape),
            Err(PortError::Unsupported(_))
        ));
    }

    #[test]
    fn quadratic_control_point_survives_and_is_bounded() {
        let mut shape = rectangle();
        shape.edges = vec![
            Edge {
                from: Point { x: 0, y: 0 },
                control: Some(Point { x: 200, y: 400 }),
                to: Point { x: 400, y: 0 },
                fill_left: 0,
                fill_right: 1,
                line: 0,
            },
            Edge {
                from: Point { x: 400, y: 0 },
                control: None,
                to: Point { x: 0, y: 0 },
                fill_left: 0,
                fill_right: 1,
                line: 0,
            },
        ];
        let path = shape_fill_paths(&shape).unwrap().pop().unwrap().geometry;
        assert!(path.commands.contains(&PathCommand::QuadTo {
            cx: 127000,
            cy: 254000,
            x: 254000,
            y: 0
        }));
        assert_eq!(path.bounds.height, 254000);
    }
}
