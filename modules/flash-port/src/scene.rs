//! Статическое представление графических кадров SWF4 (без исполнения ActionScript).
//! Поддерживает узкое подмножество DefineShape, PlaceObject и ShowFrame.
//! Неподдерживаемая графика отклоняется, а не исчезает из перенесённой игры.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::{MAX_SWF_BYTES, PortError};

const MAX_SCENE_TAGS: usize = 65_536;
const MAX_FRAMES: usize = 1_024;
const MAX_SHAPES: usize = 4_096;
const MAX_EDGES: usize = 16_384;
const MAX_INSTANCES: usize = 4_096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32, // SWF twips (20 twips = 1 pixel)
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub xmin: i32,
    pub xmax: i32,
    pub ymin: i32,
    pub ymax: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: i32, // twips
    pub ty: i32,
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0,
        ty: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStyle {
    pub width: u16,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: Point,
    pub control: Option<Point>, // quadratic control point, if present
    pub to: Point,
    pub fill_left: u16, // style index; 0 = no fill
    pub fill_right: u16,
    pub line: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub id: u16,
    pub bounds: Rect,
    pub fills: Vec<Color>,
    pub lines: Vec<LineStyle>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub character_id: u16,
    pub depth: u16,
    pub matrix: Matrix,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub background: Color,
    pub instances: Vec<Instance>, // ordered by ascending SWF depth
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub stage: Rect,
    pub frame_rate: f64,
    pub shapes: BTreeMap<u16, Shape>,
    pub frames: Vec<Frame>,
}

// MSB-first bit reader for RECT, MATRIX and SHAPERECORD structures.
struct Bits<'a> {
    input: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, bit: 0 }
    }

    fn unsigned(&mut self, width: usize) -> Result<u32, PortError> {
        if width > 32
            || self
                .bit
                .checked_add(width)
                .is_none_or(|end| end > self.input.len() * 8)
        {
            return Err(PortError::Malformed("truncated bit field"));
        }
        let mut value = 0_u32;
        for _ in 0..width {
            let byte = self.input[self.bit / 8];
            value = (value << 1) | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }

    fn signed(&mut self, width: usize) -> Result<i32, PortError> {
        let raw = self.unsigned(width)?;
        if width == 0 {
            return Ok(0);
        }
        if width == 32 {
            return Ok(raw as i32);
        }
        let sign = 1_u32 << (width - 1);
        Ok(((raw ^ sign) as i64 - i64::from(sign)) as i32)
    }

    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }

    fn pos(&self) -> usize {
        self.bit.div_ceil(8)
    }

    fn u8(&mut self) -> Result<u8, PortError> {
        Ok(self.unsigned(8)? as u8)
    }

    fn u16(&mut self) -> Result<u16, PortError> {
        let lo = self.u8()?;
        let hi = self.u8()?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    fn rect(&mut self) -> Result<Rect, PortError> {
        let width = self.unsigned(5)? as usize;
        let rect = Rect {
            xmin: self.signed(width)?,
            xmax: self.signed(width)?,
            ymin: self.signed(width)?,
            ymax: self.signed(width)?,
        };
        if rect.xmin > rect.xmax || rect.ymin > rect.ymax {
            return Err(PortError::Malformed("inverted SWF RECT"));
        }
        self.align();
        Ok(rect)
    }

    fn matrix(&mut self) -> Result<Matrix, PortError> {
        let mut matrix = Matrix::IDENTITY;
        if self.unsigned(1)? != 0 {
            let n = self.unsigned(5)? as usize;
            matrix.a = f64::from(self.signed(n)?) / 65_536.0;
            matrix.d = f64::from(self.signed(n)?) / 65_536.0;
        }
        if self.unsigned(1)? != 0 {
            let n = self.unsigned(5)? as usize;
            matrix.b = f64::from(self.signed(n)?) / 65_536.0;
            matrix.c = f64::from(self.signed(n)?) / 65_536.0;
        }
        let n = self.unsigned(5)? as usize;
        matrix.tx = self.signed(n)?;
        matrix.ty = self.signed(n)?;
        self.align();
        Ok(matrix)
    }
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, PortError> {
    let pair = data
        .get(at..at + 2)
        .ok_or(PortError::Malformed("truncated u16"))?;
    Ok(u16::from_le_bytes([pair[0], pair[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, PortError> {
    let quad = data
        .get(at..at + 4)
        .ok_or(PortError::Malformed("truncated u32"))?;
    Ok(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
}

fn next_point(point: Point, dx: i32, dy: i32) -> Result<Point, PortError> {
    Ok(Point {
        x: point
            .x
            .checked_add(dx)
            .ok_or(PortError::Malformed("shape x overflow"))?,
        y: point
            .y
            .checked_add(dy)
            .ok_or(PortError::Malformed("shape y overflow"))?,
    })
}

fn read_shape(body: &[u8]) -> Result<Shape, PortError> {
    if body.len() < 5 {
        return Err(PortError::Malformed("short DefineShape"));
    }
    let id = u16_at(body, 0)?;
    if id == 0 {
        return Err(PortError::Malformed("zero shape ID"));
    }
    let mut reader = Bits::new(&body[2..]);
    let bounds = reader.rect()?;

    // DefineShape1 only: solid RGB fills and simple line styles.
    let nfill = reader.u8()? as usize;
    let mut fills = Vec::with_capacity(nfill);
    for _ in 0..nfill {
        if reader.u8()? != 0 {
            return Err(PortError::Unsupported("gradient or bitmap fill"));
        }
        fills.push(Color {
            r: reader.u8()?,
            g: reader.u8()?,
            b: reader.u8()?,
        });
    }
    let nline = reader.u8()? as usize;
    let mut lines = Vec::with_capacity(nline);
    for _ in 0..nline {
        let width = reader.u16()?;
        let color = Color {
            r: reader.u8()?,
            g: reader.u8()?,
            b: reader.u8()?,
        };
        lines.push(LineStyle { width, color });
    }
    let num_fill_bits = reader.unsigned(4)? as usize;
    let num_line_bits = reader.unsigned(4)? as usize;
    let mut position = Point::default();
    let (mut left, mut right, mut line) = (0_u16, 0_u16, 0_u16);
    let mut edges = Vec::new();
    let mut ended = false;
    for _ in 0..MAX_EDGES {
        if reader.unsigned(1)? != 0 {
            let straight = reader.unsigned(1)? != 0;
            let n = reader.unsigned(4)? as usize + 2;
            let (control, end) = if straight {
                let general = reader.unsigned(1)? != 0;
                let (dx, dy) = if general {
                    (reader.signed(n)?, reader.signed(n)?)
                } else if reader.unsigned(1)? != 0 {
                    (0, reader.signed(n)?)
                } else {
                    (reader.signed(n)?, 0)
                };
                (None, next_point(position, dx, dy)?)
            } else {
                let control = next_point(position, reader.signed(n)?, reader.signed(n)?)?;
                let end = next_point(control, reader.signed(n)?, reader.signed(n)?)?;
                (Some(control), end)
            };
            edges.push(Edge {
                from: position,
                control,
                to: end,
                fill_left: left,
                fill_right: right,
                line,
            });
            position = end;
        } else {
            let flags = reader.unsigned(5)? as u8;
            if flags == 0 {
                ended = true;
                break;
            }
            // NewStyles shifts style indexes and may require new fill tables:
            // never silently strip it out of the geometry.
            if flags & 0x10 != 0 {
                return Err(PortError::Unsupported("ShapeRecord NewStyles"));
            }
            if flags & 0x01 != 0 {
                let n = reader.unsigned(5)? as usize;
                position = Point {
                    x: reader.signed(n)?,
                    y: reader.signed(n)?,
                };
            }
            if flags & 0x02 != 0 {
                left = reader.unsigned(num_fill_bits)? as u16;
            }
            if flags & 0x04 != 0 {
                right = reader.unsigned(num_fill_bits)? as u16;
            }
            if flags & 0x08 != 0 {
                line = reader.unsigned(num_line_bits)? as u16;
            }
            if usize::from(left) > fills.len()
                || usize::from(right) > fills.len()
                || usize::from(line) > lines.len()
            {
                return Err(PortError::Malformed("shape style index outside table"));
            }
        }
    }
    if !ended {
        return Err(PortError::Malformed(
            "shape lacks EndShapeRecord or has too many edges",
        ));
    }
    reader.align();
    if reader.pos() != body.len() - 2 {
        return Err(PortError::Malformed("unexpected bytes after shape"));
    }
    Ok(Shape {
        id,
        bounds,
        fills,
        lines,
        edges,
    })
}

fn parse_placement(
    body: &[u8],
    is_v2: bool,
    display: &mut BTreeMap<u16, Instance>,
) -> Result<(), PortError> {
    if is_v2 {
        let flags = *body
            .first()
            .ok_or(PortError::Malformed("short PlaceObject2"))?;
        if flags & !0x07 != 0 {
            return Err(PortError::Unsupported(
                "PlaceObject2 extra flags (name, color, clip, ratio or actions)",
            ));
        }
        let depth = u16_at(body, 1)?;
        let is_move = flags & 0x01 != 0;
        let has_character = flags & 0x02 != 0;
        let has_matrix = flags & 0x04 != 0;
        if !has_character && !is_move {
            return Err(PortError::Malformed(
                "PlaceObject2 has no character or move",
            ));
        }
        let mut offset = 3;
        let character_id = if has_character {
            let id = u16_at(body, offset)?;
            offset += 2;
            id
        } else {
            display
                .get(&depth)
                .ok_or(PortError::Malformed("modifying absent depth"))?
                .character_id
        };
        let matrix = if has_matrix {
            let mut b = Bits::new(&body[offset..]);
            let matrix = b.matrix()?;
            offset += b.pos();
            matrix
        } else if is_move {
            display
                .get(&depth)
                .ok_or(PortError::Malformed("moving absent depth"))?
                .matrix
        } else {
            Matrix::IDENTITY
        };
        if is_move && !display.contains_key(&depth) {
            return Err(PortError::Malformed("moving absent depth"));
        }
        if offset != body.len() {
            return Err(PortError::Malformed("unparsed PlaceObject2 tail"));
        }
        display.insert(
            depth,
            Instance {
                character_id,
                depth,
                matrix,
            },
        );
    } else {
        let character_id = u16_at(body, 0)?;
        let depth = u16_at(body, 2)?;
        let mut b = Bits::new(&body[4..]);
        let matrix = b.matrix()?;
        if b.pos() + 4 != body.len() {
            return Err(PortError::Unsupported("PlaceObject color transform"));
        }
        display.insert(
            depth,
            Instance {
                character_id,
                depth,
                matrix,
            },
        );
    }
    Ok(())
}

/// Статический импорт сцены; AVM1-код и неизвестные теги явно отклоняются.
/// Содержимое не исполняется. В отличие от parse_swf, допускается несколько кадров.
pub fn parse_scene(bytes: &[u8]) -> Result<Scene, PortError> {
    if bytes.len() < 12 || bytes.len() > MAX_SWF_BYTES {
        return Err(PortError::Malformed("invalid or oversized SWF"));
    }
    if &bytes[..3] != b"FWS" || bytes[3] != 4 {
        return Err(PortError::Unsupported(
            "scene importer accepts only uncompressed SWF4",
        ));
    }
    if u32_at(bytes, 4)? as usize != bytes.len() {
        return Err(PortError::Malformed("declared size does not match input"));
    }
    let mut header = Bits::new(&bytes[8..]);
    let stage = header.rect()?;
    let mut pos = 8 + header.pos();
    let frame_rate = f64::from(u16_at(bytes, pos)?) / 256.0;
    pos += 2;
    let declared_frames = u16_at(bytes, pos)? as usize;
    pos += 2;
    if declared_frames == 0 || declared_frames > MAX_FRAMES {
        return Err(PortError::Unsupported("scene frame-count limit"));
    }
    let mut shapes = BTreeMap::new();
    let mut display: BTreeMap<u16, Instance> = BTreeMap::new();
    let mut frames = Vec::new();
    let mut background = Color::WHITE;
    let mut found_end = false;
    for _ in 0..MAX_SCENE_TAGS {
        let tag = u16_at(bytes, pos)?;
        pos += 2;
        let kind = tag >> 6;
        let mut len = usize::from(tag & 0x3f);
        if len == 63 {
            len = u32_at(bytes, pos)? as usize;
            pos += 4;
        }
        let end = pos
            .checked_add(len)
            .ok_or(PortError::Malformed("tag offset overflow"))?;
        let body = bytes
            .get(pos..end)
            .ok_or(PortError::Malformed("truncated scene tag"))?;
        pos = end;
        match kind {
            0 => {
                if len != 0 || pos != bytes.len() {
                    return Err(PortError::Malformed("invalid final End tag"));
                }
                found_end = true;
                break;
            }
            1 => {
                if len != 0 || frames.len() >= declared_frames {
                    return Err(PortError::Malformed("invalid ShowFrame"));
                }
                frames.push(Frame {
                    background,
                    instances: display.values().cloned().collect(),
                });
            }
            2 => {
                if shapes.len() >= MAX_SHAPES {
                    return Err(PortError::Unsupported("too many shape definitions"));
                }
                let shape = read_shape(body)?;
                if shapes.insert(shape.id, shape).is_some() {
                    return Err(PortError::Malformed("duplicate shape ID"));
                }
            }
            4 | 26 => {
                parse_placement(body, kind == 26, &mut display)?;
                if display.len() > MAX_INSTANCES {
                    return Err(PortError::Unsupported("too many placed objects"));
                }
            }
            5 => {
                if body.len() != 4 {
                    return Err(PortError::Malformed("invalid RemoveObject"));
                }
                let id = u16_at(body, 0)?;
                let depth = u16_at(body, 2)?;
                if display.get(&depth).is_none_or(|o| o.character_id != id) {
                    return Err(PortError::Malformed("RemoveObject ID/depth mismatch"));
                }
                display.remove(&depth);
            }
            28 => {
                if body.len() != 2 || display.remove(&u16_at(body, 0)?).is_none() {
                    return Err(PortError::Malformed("invalid RemoveObject2"));
                }
            }
            9 => {
                if body.len() != 3 {
                    return Err(PortError::Malformed("invalid background color"));
                }
                background = Color {
                    r: body[0],
                    g: body[1],
                    b: body[2],
                };
            }
            12 => {
                return Err(PortError::Unsupported(
                    "scene ActionScript must be linked to Rust codegen",
                ));
            }
            _ => return Err(PortError::UnsupportedTag(kind)),
        }
    }
    if !found_end || frames.len() != declared_frames {
        return Err(PortError::Malformed(
            "missing End or incorrect number of frames",
        ));
    }
    for frame in &frames {
        for instance in &frame.instances {
            if !shapes.contains_key(&instance.character_id) {
                return Err(PortError::Malformed(
                    "placed character has no shape definition",
                ));
            }
        }
    }
    Ok(Scene {
        stage,
        frame_rate,
        shapes,
        frames,
    })
}

#[derive(Clone, Copy)]
pub(crate) struct DirectedEdge {
    pub(crate) from: Point,
    pub(crate) control: Option<Point>,
    pub(crate) to: Point,
}

/// Resolve SWF fill sides into closed, oriented contours. The same logic is
/// shared by reference SVG and the Chaptera path producer; neither may invent
/// an edge ordering or silently drop an unclosed contour.
pub(crate) fn contours_for_fill(
    shape: &Shape,
    fill_id: u16,
) -> Result<Vec<Vec<DirectedEdge>>, PortError> {
    let mut pending = Vec::new();
    for edge in &shape.edges {
        if edge.fill_left == fill_id {
            pending.push(DirectedEdge {
                from: edge.to,
                control: edge.control,
                to: edge.from,
            });
        }
        if edge.fill_right == fill_id {
            pending.push(DirectedEdge {
                from: edge.from,
                control: edge.control,
                to: edge.to,
            });
        }
    }
    let mut contours = Vec::new();
    while !pending.is_empty() {
        let start = pending[0].from;
        let mut cursor = start;
        let mut contour = Vec::new();
        loop {
            let mut matches = pending.iter().enumerate().filter(|(_, e)| e.from == cursor);
            let Some((next, _)) = matches.next() else {
                return Err(PortError::Unsupported("unclosed fill contour"));
            };
            if matches.next().is_some() {
                return Err(PortError::Unsupported("ambiguous fill contour junction"));
            }
            let edge = pending.remove(next);
            cursor = edge.to;
            contour.push(edge);
            if cursor == start {
                break;
            }
        }
        contours.push(contour);
    }
    Ok(contours)
}

fn color_hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Контрольный статический SVG конкретного кадра. Не является игровым
/// проигрывателем; нужен для сопоставления геометрии перед Vello.
pub fn emit_svg_frame(scene: &Scene, frame_index: usize) -> Result<String, PortError> {
    let frame = scene
        .frames
        .get(frame_index)
        .ok_or(PortError::Malformed("frame out of range"))?;
    let stage = scene.stage;
    let width = stage.xmax - stage.xmin;
    let height = stage.ymax - stage.ymin;
    if width <= 0 || height <= 0 {
        return Err(PortError::Unsupported("zero-area stage"));
    }
    let mut svg = String::new();
    writeln!(svg, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">",
        f64::from(width) / 20.0, f64::from(height) / 20.0,
        stage.xmin, stage.ymin, width, height).expect("write String");
    writeln!(
        svg,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
        stage.xmin,
        stage.ymin,
        width,
        height,
        color_hex(frame.background)
    )
    .expect("write String");
    for instance in &frame.instances {
        let shape = scene
            .shapes
            .get(&instance.character_id)
            .ok_or(PortError::Malformed("missing shape for instance"))?;
        if !shape.lines.is_empty() {
            return Err(PortError::Unsupported("SVG line styles pending"));
        }
        let m = instance.matrix;
        writeln!(
            svg,
            "<g data-depth=\"{}\" transform=\"matrix({} {} {} {} {} {})\">",
            instance.depth, m.a, m.b, m.c, m.d, m.tx, m.ty
        )
        .expect("write String");
        for (index, fill) in shape.fills.iter().enumerate() {
            let id = (index + 1) as u16;
            for contour in contours_for_fill(shape, id)? {
                let start = contour[0].from;
                let mut path = format!("M {} {}", start.x, start.y);
                for edge in contour {
                    if let Some(control) = edge.control {
                        write!(
                            path,
                            " Q {} {} {} {}",
                            control.x, control.y, edge.to.x, edge.to.y
                        )
                        .expect("write String");
                    } else {
                        write!(path, " L {} {}", edge.to.x, edge.to.y).expect("write String");
                    }
                }
                path.push_str(" Z");
                writeln!(svg, "<path d=\"{}\" fill=\"{}\"/>", path, color_hex(*fill))
                    .expect("write String");
            }
        }
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // MSB-first fixture writer; test vectors are real SWF structures and do
    // not assume the parser's internal byte-position implementation.
    struct BitsOut {
        data: Vec<u8>,
        len: usize,
    }

    impl BitsOut {
        fn new() -> Self {
            Self {
                data: Vec::new(),
                len: 0,
            }
        }

        fn push(&mut self, bits: usize, value: i32) {
            assert!(bits <= 32);
            for shift in (0..bits).rev() {
                if self.len % 8 == 0 {
                    self.data.push(0);
                }
                let bit = ((value as u32 >> shift) & 1) as u8;
                let last = self.data.len() - 1;
                self.data[last] |= bit << (7 - self.len % 8);
                self.len += 1;
            }
        }

        fn finish(self) -> Vec<u8> {
            self.data
        }
    }

    fn rect(width: usize, x0: i32, x1: i32, y0: i32, y1: i32) -> Vec<u8> {
        let mut out = BitsOut::new();
        out.push(5, width as i32);
        for v in [x0, x1, y0, y1] {
            out.push(width, v);
        }
        out.finish()
    }

    fn matrix(tx: i32, ty: i32) -> Vec<u8> {
        let mut out = BitsOut::new();
        out.push(1, 0); // no scale
        out.push(1, 0); // no rotation
        out.push(5, 12); // signed translation
        out.push(12, tx);
        out.push(12, ty);
        out.finish()
    }

    fn shape() -> Vec<u8> {
        let mut buf = 7_u16.to_le_bytes().to_vec();
        buf.extend_from_slice(&rect(10, 0, 400, 0, 200));
        buf.extend_from_slice(&[1, 0, 255, 0, 0]); // one red solid fill
        buf.push(0); // zero line styles
        let mut out = BitsOut::new();
        out.push(4, 1); // one fill index bit
        out.push(4, 0); // zero stroke bits
        out.push(1, 0); // style-change record
        out.push(5, 0x05); // move-to + fillStyle1
        out.push(5, 1); // move-to uses signed 1-bit coordinates
        out.push(1, 0);
        out.push(1, 0);
        out.push(1, 1); // fillStyle1 index
        for (is_vertical, displacement) in [(0, 400), (1, 200), (0, -400), (1, -200)] {
            out.push(1, 1); // edge record
            out.push(1, 1); // straight
            out.push(4, 8); // signed delta, 10 bits
            out.push(1, 0); // axis-aligned
            out.push(1, is_vertical);
            out.push(10, displacement);
        }
        out.push(1, 0); // EndShapeRecord
        out.push(5, 0);
        buf.extend_from_slice(&out.finish());
        buf
    }

    fn tag(kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        if data.len() < 63 {
            out.extend_from_slice(&((kind << 6) | data.len() as u16).to_le_bytes());
        } else {
            out.extend_from_slice(&((kind << 6) | 63).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(data);
        out
    }

    fn scene_swf() -> Vec<u8> {
        let mut swf = b"FWS\x04\0\0\0\0".to_vec();
        swf.extend_from_slice(&rect(13, 0, 3000, 0, 2000));
        swf.extend_from_slice(&(24_u16 << 8).to_le_bytes());
        swf.extend_from_slice(&3_u16.to_le_bytes());
        swf.extend_from_slice(&tag(9, &[230, 230, 230]));
        swf.extend_from_slice(&tag(2, &shape()));
        let mut placement = [7_u16.to_le_bytes(), 1_u16.to_le_bytes()].concat();
        placement.extend_from_slice(&matrix(600, 400));
        swf.extend_from_slice(&tag(4, &placement));
        swf.extend_from_slice(&tag(1, &[]));
        let mut move_object = vec![0x05];
        move_object.extend_from_slice(&1_u16.to_le_bytes());
        move_object.extend_from_slice(&matrix(1000, 800));
        swf.extend_from_slice(&tag(26, &move_object));
        swf.extend_from_slice(&tag(1, &[]));
        swf.extend_from_slice(&tag(28, &1_u16.to_le_bytes()));
        swf.extend_from_slice(&tag(1, &[]));
        swf.extend_from_slice(&tag(0, &[]));
        let size = swf.len() as u32;
        swf[4..8].copy_from_slice(&size.to_le_bytes());
        swf
    }

    #[test]
    fn reads_vector_edges_and_three_frame_snapshots() {
        let scene = parse_scene(&scene_swf()).unwrap();
        assert_eq!(scene.frame_rate, 24.0);
        assert_eq!(scene.stage.xmax, 3000);
        assert_eq!(scene.shapes.len(), 1);
        let shape = &scene.shapes[&7];
        assert_eq!(shape.edges.len(), 4);
        assert_eq!(shape.fills, vec![Color { r: 255, g: 0, b: 0 }]);
        assert_eq!(shape.edges[0].from, Point { x: 0, y: 0 });
        assert_eq!(shape.edges[0].to, Point { x: 400, y: 0 });
        assert_eq!(scene.frames.len(), 3);
        assert_eq!(scene.frames[0].instances[0].matrix.tx, 600);
        assert_eq!(scene.frames[1].instances[0].matrix.tx, 1000);
        assert!(scene.frames[2].instances.is_empty());
    }

    #[test]
    fn exports_true_solid_shape_to_static_svg() {
        let scene = parse_scene(&scene_swf()).unwrap();
        let svg0 = emit_svg_frame(&scene, 0).unwrap();
        assert!(svg0.contains("matrix(1 0 0 1 600 400)"));
        assert!(svg0.contains("M 0 0 L 400 0 L 400 200 L 0 200 L 0 0 Z"));
        assert!(svg0.contains("fill=\"#ff0000\""));
        let svg1 = emit_svg_frame(&scene, 1).unwrap();
        assert!(svg1.contains("matrix(1 0 0 1 1000 800)"));
        let svg2 = emit_svg_frame(&scene, 2).unwrap();
        assert!(!svg2.contains("<path"));
    }

    #[test]
    fn malformed_or_unsupported_scene_fails_closed() {
        let mut broken = scene_swf();
        broken[0] = b'C';
        assert!(matches!(
            parse_scene(&broken),
            Err(PortError::Unsupported(_))
        ));
        let mut broken = scene_swf();
        broken.pop();
        assert!(matches!(parse_scene(&broken), Err(PortError::Malformed(_))));
        let mut broken = scene_swf();
        // First tag is SetBackgroundColor. Replace it with an unsupported DoAction.
        let header_len = 8 + rect(13, 0, 3000, 0, 2000).len() + 4;
        let action_header = (12_u16 << 6) | 3;
        broken[header_len..header_len + 2].copy_from_slice(&action_header.to_le_bytes());
        assert!(matches!(
            parse_scene(&broken),
            Err(PortError::Unsupported(_))
        ));
    }

    #[test]
    fn unsupported_gradient_and_extra_place_flags_are_not_dropped() {
        let mut s = shape();
        let fill_kind_offset = 2 + rect(10, 0, 400, 0, 200).len() + 1;
        s[fill_kind_offset] = 0x10;
        assert!(matches!(read_shape(&s), Err(PortError::Unsupported(_))));
        let mut display = BTreeMap::new();
        assert!(matches!(
            parse_placement(&[0x20, 1, 0], true, &mut display),
            Err(PortError::Unsupported(_))
        ));
    }
}
