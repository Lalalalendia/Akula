//! Минимальный статический перенос SWF4 в самостоятельный Rust-код.
//! Это НЕ проигрыватель Flash и не полный импортёр SWF.

use std::fmt::{self, Write};

pub mod chaptera_path;
pub mod scene;

/// Жёсткий предел для исследовательского импортёра. Это не песочница ОС.
pub const MAX_SWF_BYTES: usize = 8 * 1024 * 1024;
const MAX_ACTIONS: usize = 16_384;
const MAX_TAGS: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    Malformed(&'static str),
    Unsupported(&'static str),
    UnsupportedTag(u16),
    UnsupportedAction(u8),
    UnsupportedPush(u8),
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(s) => write!(f, "malformed SWF: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported SWF feature: {s}"),
            Self::UnsupportedTag(t) => write!(f, "unsupported SWF tag: {t}"),
            Self::UnsupportedAction(a) => write!(f, "unsupported AVM1 action: 0x{a:02x}"),
            Self::UnsupportedPush(p) => write!(f, "unsupported AVM1 push type: {p}"),
        }
    }
}

impl std::error::Error for PortError {}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Null,
    Undefined,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Push(LegacyValue),
    Add,
    Trace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Movie {
    pub version: u8,
    pub frame_rate: f64,
    pub frame_count: u16,
    pub actions: Vec<Action>,
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, PortError> {
    let end = offset
        .checked_add(2)
        .ok_or(PortError::Malformed("offset overflow"))?;
    let b = bytes
        .get(offset..end)
        .ok_or(PortError::Malformed("truncated u16"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, PortError> {
    let end = offset
        .checked_add(4)
        .ok_or(PortError::Malformed("offset overflow"))?;
    let b = bytes
        .get(offset..end)
        .ok_or(PortError::Malformed("truncated u32"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Пока поддерживается только несжатый FWS версии 4: расширение обязательно
/// должно быть явным, чтобы неизвестные конструкции не пропадали молча.
pub fn parse_swf(bytes: &[u8]) -> Result<Movie, PortError> {
    if bytes.len() < 9 || bytes.len() > MAX_SWF_BYTES {
        return Err(PortError::Malformed("invalid or oversized SWF"));
    }
    match &bytes[..3] {
        b"FWS" => {}
        b"CWS" | b"ZWS" => return Err(PortError::Unsupported("compressed SWF")),
        _ => return Err(PortError::Malformed("invalid SWF signature")),
    }
    let version = bytes[3];
    if version != 4 {
        return Err(PortError::Unsupported("only SWF version 4 is supported"));
    }
    let declared_size = le_u32(bytes, 4)? as usize;
    if declared_size != bytes.len() {
        return Err(PortError::Malformed("declared size does not match input"));
    }
    let nbits = usize::from(bytes[8] >> 3);
    let rect_len = (5 + 4 * nbits).div_ceil(8);
    let mut pos = 8 + rect_len;
    if pos + 4 > bytes.len() {
        return Err(PortError::Malformed("truncated stage header"));
    }
    let frame_rate = f64::from(le_u16(bytes, pos)?) / 256.0;
    pos += 2;
    let frame_count = le_u16(bytes, pos)?;
    pos += 2;
    if frame_count != 1 {
        return Err(PortError::Unsupported(
            "only one-frame movies are supported",
        ));
    }

    let mut actions = Vec::new();
    let mut frames = 0;
    let mut scripts = 0;
    let mut found_end = false;
    for _ in 0..MAX_TAGS {
        if pos == bytes.len() {
            break;
        }
        let header = le_u16(bytes, pos)?;
        pos += 2;
        let tag = header >> 6;
        let mut len = usize::from(header & 0x3f);
        if len == 0x3f {
            len = le_u32(bytes, pos)? as usize;
            pos += 4;
        }
        let end = pos
            .checked_add(len)
            .ok_or(PortError::Malformed("tag offset overflow"))?;
        let body = bytes
            .get(pos..end)
            .ok_or(PortError::Malformed("truncated tag"))?;
        pos = end;
        match tag {
            0 => {
                if !body.is_empty() || pos != bytes.len() {
                    return Err(PortError::Malformed("invalid End tag"));
                }
                found_end = true;
                break;
            }
            1 => {
                if !body.is_empty() {
                    return Err(PortError::Malformed("ShowFrame must be empty"));
                }
                frames += 1;
                if frames > 1 {
                    return Err(PortError::Unsupported("multiple ShowFrame tags"));
                }
            }
            9 => {
                if body.len() != 3 {
                    return Err(PortError::Malformed("invalid background color"));
                }
                // Console-only P0: цвет будет частью будущей SceneIR.
            }
            12 => {
                if frames != 0 || scripts != 0 {
                    return Err(PortError::Unsupported("multiple or late DoAction tags"));
                }
                scripts += 1;
                parse_actions(body, &mut actions)?;
            }
            _ => return Err(PortError::UnsupportedTag(tag)),
        }
    }
    if !found_end {
        return Err(PortError::Malformed(
            "missing End tag or excessive tag count",
        ));
    }
    if frames != 1 {
        return Err(PortError::Malformed("missing ShowFrame"));
    }
    Ok(Movie {
        version,
        frame_rate,
        frame_count,
        actions,
    })
}

fn parse_actions(bytes: &[u8], result: &mut Vec<Action>) -> Result<(), PortError> {
    let mut pos = 0;
    let mut found_end = false;
    while pos < bytes.len() {
        if result.len() >= MAX_ACTIONS {
            return Err(PortError::Unsupported("too many AVM1 actions"));
        }
        let code = bytes[pos];
        pos += 1;
        let mut len = 0;
        if code & 0x80 != 0 {
            len = usize::from(le_u16(bytes, pos)?);
            pos += 2;
        }
        let end = pos
            .checked_add(len)
            .ok_or(PortError::Malformed("action offset overflow"))?;
        let body = bytes
            .get(pos..end)
            .ok_or(PortError::Malformed("truncated AVM1 action"))?;
        pos = end;
        match code {
            0 => {
                if pos != bytes.len() {
                    return Err(PortError::Unsupported("data after EndOfActions"));
                }
                found_end = true;
                break;
            }
            0x96 => parse_push(body, result)?,
            0x0a => result.push(Action::Add),
            0x26 => result.push(Action::Trace),
            _ => return Err(PortError::UnsupportedAction(code)),
        }
    }
    if !found_end {
        return Err(PortError::Malformed("missing EndOfActions"));
    }
    Ok(())
}

fn parse_push(bytes: &[u8], actions: &mut Vec<Action>) -> Result<(), PortError> {
    let mut pos = 0;
    if bytes.is_empty() {
        return Err(PortError::Malformed("empty Push payload"));
    }
    while pos < bytes.len() {
        if actions.len() >= MAX_ACTIONS {
            return Err(PortError::Unsupported("too many AVM1 values"));
        }
        let kind = bytes[pos];
        pos += 1;
        let value = match kind {
            0 => {
                let tail = &bytes[pos..];
                let count = tail
                    .iter()
                    .position(|c| *c == 0)
                    .ok_or(PortError::Malformed("unterminated SWF string"))?;
                let str_bytes = &tail[..count];
                let value = std::str::from_utf8(str_bytes)
                    .map_err(|_| PortError::Unsupported("non-UTF-8 legacy string"))?;
                pos += count + 1;
                LegacyValue::Text(value.to_owned())
            }
            1 => {
                let end = pos
                    .checked_add(4)
                    .ok_or(PortError::Malformed("float offset overflow"))?;
                let b = bytes
                    .get(pos..end)
                    .ok_or(PortError::Malformed("truncated float"))?;
                pos = end;
                LegacyValue::Number(f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
            }
            2 => LegacyValue::Null,
            3 => LegacyValue::Undefined,
            5 => {
                let byte = *bytes
                    .get(pos)
                    .ok_or(PortError::Malformed("truncated boolean"))?;
                pos += 1;
                LegacyValue::Boolean(byte != 0)
            }
            7 => {
                let number = le_u32(bytes, pos)? as i32;
                pos += 4;
                LegacyValue::Number(f64::from(number))
            }
            _ => return Err(PortError::UnsupportedPush(kind)),
        };
        actions.push(Action::Push(value));
    }
    Ok(())
}

const RUST_PROLOGUE: &str = r#"// Generated by akula-flash-port. SWF is not needed at runtime.
// Narrow SWF4/AVM1 compatibility helpers; not a Flash virtual machine.
#[derive(Clone)]
enum LegacyValue {
    Number(f64),
    Text(&'static str),
    Boolean(bool),
    Null,
    Undefined,
}
fn number(value: &LegacyValue) -> f64 {
    match value {
        LegacyValue::Number(n) => *n,
        LegacyValue::Text(s) => {
            if s.trim().is_empty() { 0.0 }
            else { s.trim().parse::<f64>().unwrap_or(f64::NAN) }
        }
        LegacyValue::Boolean(b) => if *b { 1.0 } else { 0.0 },
        LegacyValue::Null => 0.0,
        LegacyValue::Undefined => f64::NAN,
    }
}
fn trace(value: &LegacyValue) -> String {
    match value {
        LegacyValue::Number(n) if n.is_nan() => "NaN".to_owned(),
        LegacyValue::Number(n) if *n == f64::INFINITY => "Infinity".to_owned(),
        LegacyValue::Number(n) if *n == f64::NEG_INFINITY => "-Infinity".to_owned(),
        LegacyValue::Number(n) => n.to_string(),
        LegacyValue::Text(s) => (*s).to_owned(),
        LegacyValue::Boolean(b) => b.to_string(),
        LegacyValue::Null => "null".to_owned(),
        LegacyValue::Undefined => "undefined".to_owned(),
    }
}
fn main() {
"#;

/// Прямое разворачивание инструкций в Rust-операторы: во время исполнения
/// сгенерированная программа не читает SWF и не интерпретирует AVM1 байткод.
pub fn emit_rust(movie: &Movie) -> Result<String, PortError> {
    let mut out = RUST_PROLOGUE.to_owned();
    let mut stack: Vec<usize> = Vec::new();
    let mut next_var = 0_usize;
    for action in &movie.actions {
        match action {
            Action::Push(value) => {
                let literal = match value {
                    LegacyValue::Text(s) => format!("LegacyValue::Text({s:?})"),
                    LegacyValue::Number(n) if n.is_nan() => {
                        "LegacyValue::Number(f64::NAN)".to_owned()
                    }
                    LegacyValue::Number(n) if *n == f64::INFINITY => {
                        "LegacyValue::Number(f64::INFINITY)".to_owned()
                    }
                    LegacyValue::Number(n) if *n == f64::NEG_INFINITY => {
                        "LegacyValue::Number(f64::NEG_INFINITY)".to_owned()
                    }
                    LegacyValue::Number(n) => format!("LegacyValue::Number({n:?})"),
                    LegacyValue::Boolean(b) => format!("LegacyValue::Boolean({b})"),
                    LegacyValue::Null => "LegacyValue::Null".to_owned(),
                    LegacyValue::Undefined => "LegacyValue::Undefined".to_owned(),
                };
                writeln!(out, "    let v{next_var} = {literal};").expect("writing to String");
                stack.push(next_var);
                next_var += 1;
            }
            Action::Add => {
                let right = stack
                    .pop()
                    .ok_or(PortError::Malformed("ADD stack underflow"))?;
                let left = stack
                    .pop()
                    .ok_or(PortError::Malformed("ADD stack underflow"))?;
                writeln!(out, "    let v{next_var} = LegacyValue::Number(number(&v{left}) + number(&v{right}));")
                    .expect("writing to String");
                stack.push(next_var);
                next_var += 1;
            }
            Action::Trace => {
                let value = stack
                    .pop()
                    .ok_or(PortError::Malformed("TRACE stack underflow"))?;
                writeln!(out, "    println!(\"{{}}\", trace(&v{value}));")
                    .expect("writing to String");
            }
        }
    }
    out.push_str("}\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn swf4(actions: &[u8]) -> Vec<u8> {
        // Same stage RECT and 24 fps used by SWFRecomp's trace_swf_4 fixture.
        let mut swf = b"FWS\x04\0\0\0\0".to_vec();
        swf.extend_from_slice(&[0x78, 0x00, 0x0f, 0xa0, 0x00, 0x00, 0x0f, 0xa0, 0x00]);
        swf.extend_from_slice(&(24_u16 << 8).to_le_bytes());
        swf.extend_from_slice(&1_u16.to_le_bytes());
        swf.extend_from_slice(&((12_u16 << 6) | 0x3f).to_le_bytes());
        swf.extend_from_slice(&(actions.len() as u32).to_le_bytes());
        swf.extend_from_slice(actions);
        swf.extend_from_slice(&(1_u16 << 6).to_le_bytes());
        swf.extend_from_slice(&0_u16.to_le_bytes());
        let len = swf.len() as u32;
        swf[4..8].copy_from_slice(&len.to_le_bytes());
        swf
    }

    fn trace_actions() -> Vec<u8> {
        let text = b"sup from SWF 4";
        let mut a = vec![0x96];
        a.extend_from_slice(&((text.len() + 2) as u16).to_le_bytes());
        a.push(0);
        a.extend_from_slice(text);
        a.push(0);
        a.extend_from_slice(&[0x26, 0]);
        a
    }

    fn push_f32(actions: &mut Vec<u8>, value: f32) {
        actions.extend_from_slice(&[0x96, 5, 0, 1]);
        actions.extend_from_slice(&value.to_le_bytes());
    }

    fn emit_output(actions: &[u8]) -> String {
        emit_rust(&parse_swf(&swf4(actions)).expect("SWF must parse")).expect("must lower")
    }

    #[test]
    fn trace_from_real_swf4_structure() {
        let movie = parse_swf(&swf4(&trace_actions())).unwrap();
        assert_eq!(movie.version, 4);
        assert_eq!(movie.frame_count, 1);
        assert_eq!(movie.frame_rate, 24.0);
        assert_eq!(movie.actions.len(), 2);
        let rust = emit_rust(&movie).unwrap();
        assert!(rust.contains("sup from SWF 4"));
        assert!(!rust.contains("include_bytes!"));
    }

    #[test]
    fn generated_rust_compiles_and_runs_without_swf() {
        let rust = emit_output(&trace_actions());
        let dir = std::env::temp_dir().join(format!(
            "akula-flash-port-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("generated.rs");
        let exe = dir.join(format!("generated{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&src, rust).unwrap();
        let tool = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let build = Command::new(tool)
            .args(["--edition=2024", "-o"])
            .arg(&exe)
            .arg(&src)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "generated code failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        let run = Command::new(&exe).output().unwrap();
        assert!(run.status.success());
        assert_eq!(String::from_utf8(run.stdout).unwrap(), "sup from SWF 4\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_is_lowered_as_static_statements_not_bytecode_loop() {
        let mut actions = Vec::new();
        push_f32(&mut actions, 3.0);
        push_f32(&mut actions, 5.0);
        actions.extend_from_slice(&[0x0a, 0x26, 0]);
        let rust = emit_output(&actions);
        assert!(rust.contains("number(&v0) + number(&v1)"));
        assert!(rust.contains("println!"));
        assert!(!rust.contains("match pc"));
    }

    #[test]
    fn malformed_and_unsupported_inputs_fail_closed() {
        let mut normal = swf4(&trace_actions());
        let mut wrong_len = normal.clone();
        wrong_len[4] = 0;
        assert!(matches!(
            parse_swf(&wrong_len),
            Err(PortError::Malformed(_))
        ));

        normal[0] = b'C';
        assert_eq!(
            parse_swf(&normal),
            Err(PortError::Unsupported("compressed SWF"))
        );

        let mut unknown_tag = swf4(&trace_actions());
        let tag_start = 8 + 9 + 4;
        unknown_tag[tag_start] = 0xff;
        assert!(matches!(
            parse_swf(&unknown_tag),
            Err(PortError::UnsupportedTag(_))
        ));

        let mut unknown_action = trace_actions();
        let action_idx = unknown_action.len() - 2;
        unknown_action[action_idx] = 0x10; // intentionally unsupported JUMP
        assert!(matches!(
            parse_swf(&swf4(&unknown_action)),
            Err(PortError::UnsupportedAction(0x10))
        ));

        let mut truncated = swf4(&trace_actions());
        truncated.pop();
        assert!(matches!(
            parse_swf(&truncated),
            Err(PortError::Malformed(_))
        ));
    }

    #[test]
    fn stack_underflow_never_generates_a_fake_success() {
        let actions = [0x0a, 0]; // ADD with an empty stack
        let movie = parse_swf(&swf4(&actions)).unwrap();
        assert_eq!(
            emit_rust(&movie),
            Err(PortError::Malformed("ADD stack underflow"))
        );
    }

    #[test]
    fn rejects_non_utf8_legacy_string_instead_of_corrupting_it() {
        let actions = [0x96, 3, 0, 0, 0xff, 0, 0];
        assert_eq!(
            parse_swf(&swf4(&actions)),
            Err(PortError::Unsupported("non-UTF-8 legacy string"))
        );
    }
}
