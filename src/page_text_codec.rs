//! Compact, bounded representation of a PDF page's selectable characters.
//!
//! This is shared by the isolated PDF worker and the disposable page cache.
//! Floats are stored losslessly as little-endian IEEE 754 values.

use std::io;

use crate::model::{PageTextChar, PdfRect};

const MAGIC: &[u8; 4] = b"LPC1";
const HEADER_BYTES: usize = 8;
const MIN_RECORD_BYTES: usize = 5;
pub(crate) const MAX_ENCODED_BYTES: usize = 64_000_000;
const MAX_CHARACTERS: usize = 2_000_000;
const HAS_RECT: u8 = 1;
const HAS_FONT_SIZE: u8 = 2;
const BOLD: u8 = 4;
const ITALIC: u8 = 8;
const KNOWN_FLAGS: u8 = HAS_RECT | HAS_FONT_SIZE | BOLD | ITALIC;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn encode(chars: &[PageTextChar]) -> io::Result<Vec<u8>> {
    if chars.len() > MAX_CHARACTERS {
        return Err(invalid("Too many PDF page characters"));
    }
    let mut length = HEADER_BYTES;
    for item in chars {
        if item.rect.is_some_and(|rect| {
            ![rect.left, rect.bottom, rect.right, rect.top]
                .into_iter()
                .all(f32::is_finite)
        }) || item.font_size.is_some_and(|value| !value.is_finite())
        {
            return Err(invalid("Invalid PDF character geometry"));
        }
        length = length
            .checked_add(
                MIN_RECORD_BYTES
                    + usize::from(item.rect.is_some()) * 16
                    + usize::from(item.font_size.is_some()) * 4,
            )
            .filter(|length| *length <= MAX_ENCODED_BYTES)
            .ok_or_else(|| invalid("PDF page characters exceed the size limit"))?;
    }

    let mut output = Vec::with_capacity(length);
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&(chars.len() as u32).to_le_bytes());
    for item in chars {
        output.extend_from_slice(&(item.ch as u32).to_le_bytes());
        let flags = if item.rect.is_some() { HAS_RECT } else { 0 }
            | if item.font_size.is_some() {
                HAS_FONT_SIZE
            } else {
                0
            }
            | if item.bold { BOLD } else { 0 }
            | if item.italic { ITALIC } else { 0 };
        output.push(flags);
        if let Some(rect) = item.rect {
            for value in [rect.left, rect.bottom, rect.right, rect.top] {
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
        if let Some(value) = item.font_size {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    debug_assert_eq!(output.len(), length);
    Ok(output)
}

pub(crate) fn decode(bytes: &[u8]) -> io::Result<Vec<PageTextChar>> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_ENCODED_BYTES || &bytes[..4] != MAGIC {
        return Err(invalid("Invalid PDF character data header"));
    }
    let count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if count > MAX_CHARACTERS || count > (bytes.len() - HEADER_BYTES) / MIN_RECORD_BYTES {
        return Err(invalid("Invalid PDF character count"));
    }
    let mut position = HEADER_BYTES;
    let mut chars = Vec::with_capacity(count);
    for _ in 0..count {
        let scalar = read_u32(bytes, &mut position)?;
        let ch = char::from_u32(scalar).ok_or_else(|| invalid("Invalid PDF character"))?;
        let flags = *bytes
            .get(position)
            .ok_or_else(|| invalid("Truncated PDF character"))?;
        position += 1;
        if flags & !KNOWN_FLAGS != 0 {
            return Err(invalid("Invalid PDF character flags"));
        }
        let rect = if flags & HAS_RECT != 0 {
            Some(PdfRect {
                left: read_f32(bytes, &mut position)?,
                bottom: read_f32(bytes, &mut position)?,
                right: read_f32(bytes, &mut position)?,
                top: read_f32(bytes, &mut position)?,
            })
        } else {
            None
        };
        let font_size = if flags & HAS_FONT_SIZE != 0 {
            Some(read_f32(bytes, &mut position)?)
        } else {
            None
        };
        chars.push(PageTextChar {
            ch,
            rect,
            font_size,
            bold: flags & BOLD != 0,
            italic: flags & ITALIC != 0,
        });
    }
    if position != bytes.len() {
        return Err(invalid("Trailing PDF character data"));
    }
    Ok(chars)
}

fn read_u32(bytes: &[u8], position: &mut usize) -> io::Result<u32> {
    let end = position
        .checked_add(4)
        .ok_or_else(|| invalid("Invalid PDF character offset"))?;
    let raw: [u8; 4] = bytes
        .get(*position..end)
        .ok_or_else(|| invalid("Truncated PDF character"))?
        .try_into()
        .unwrap();
    *position = end;
    Ok(u32::from_le_bytes(raw))
}

fn read_f32(bytes: &[u8], position: &mut usize) -> io::Result<f32> {
    let value = f32::from_bits(read_u32(bytes, position)?);
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| invalid("Invalid PDF character geometry"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<PageTextChar> {
        vec![
            PageTextChar {
                ch: 'A',
                rect: Some(PdfRect::new(-1.25, 2.5, 3.75, 4.5)),
                font_size: Some(12.25),
                bold: true,
                italic: false,
            },
            PageTextChar {
                ch: '§',
                rect: None,
                font_size: None,
                bold: false,
                italic: true,
            },
            PageTextChar {
                ch: '𝄞',
                rect: Some(PdfRect::new(0.0, 1.0, 2.0, 3.0)),
                font_size: None,
                bold: true,
                italic: true,
            },
            PageTextChar {
                ch: '\n',
                rect: None,
                font_size: Some(0.0),
                bold: false,
                italic: false,
            },
        ]
    }

    #[test]
    fn round_trips_geometry_flags_unicode_and_empty() {
        let chars = sample();
        let decoded = decode(&encode(&chars).unwrap()).unwrap();
        assert_eq!(chars.len(), decoded.len());
        for (a, b) in chars.iter().zip(decoded.iter()) {
            assert_eq!(
                (a.ch, a.rect, a.font_size, a.bold, a.italic),
                (b.ch, b.rect, b.font_size, b.bold, b.italic)
            );
        }
        assert!(decode(&encode(&[]).unwrap()).unwrap().is_empty());
        assert!(encode(&chars).unwrap().len() < serde_json::to_vec(&chars).unwrap().len());
    }

    #[test]
    fn rejects_malformed_and_truncated_records() {
        let valid = encode(&sample()).unwrap();
        for end in 0..valid.len() {
            assert!(
                decode(&valid[..end]).is_err(),
                "accepted truncation at {end}"
            );
        }
        let mut extra = valid.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        let mut invalid_count = valid.clone();
        invalid_count[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&invalid_count).is_err());
        let mut invalid_char = valid.clone();
        invalid_char[8..12].copy_from_slice(&0xd800u32.to_le_bytes());
        assert!(decode(&invalid_char).is_err());
        let mut invalid_flags = valid.clone();
        invalid_flags[12] |= 0x80;
        assert!(decode(&invalid_flags).is_err());
        let mut invalid_float = valid;
        invalid_float[13..17].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&invalid_float).is_err());
        let mut invalid_source = sample();
        invalid_source[0].font_size = Some(f32::INFINITY);
        assert!(encode(&invalid_source).is_err());
    }
}
