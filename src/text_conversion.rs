use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use lopdf::{
    Document, Object, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use quick_xml::{Reader, events::Event};
use zip::ZipArchive;

const PAGE_WIDTH: f32 = 612.0;
const PAGE_HEIGHT: f32 = 792.0;
const MARGIN_LEFT: f32 = 54.0;
const MARGIN_TOP: f32 = 54.0;
const MARGIN_BOTTOM: f32 = 54.0;
const BODY_FONT_SIZE: f32 = 11.0;
const LINE_HEIGHT: f32 = 14.0;
const MAX_COLLISION_ATTEMPTS: usize = 1000;

const CONVERTIBLE_EXTENSIONS: &[&str] = &[
    "docx", "md", "markdown", "txt", "text", "log", "csv", "json",
];

#[derive(Debug, Clone)]
pub struct ConversionOutput {
    pub destination: PathBuf,
}

pub fn is_convertible_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            CONVERTIBLE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

pub fn convert_source_to_pdf(source: &Path) -> Result<PathBuf> {
    let source = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let text = extract_source_text(&source)?;
    let destination = next_pdf_path(&source)?;
    write_text_pdf(&text, &source, &destination)?;
    Ok(destination)
}

pub fn convert_sources_to_pdf(paths: &[PathBuf]) -> Result<Vec<ConversionOutput>> {
    let mut outputs = Vec::new();
    for path in paths {
        let destination = convert_source_to_pdf(path)
            .with_context(|| format!("failed to convert {}", path.display()))?;
        outputs.push(ConversionOutput { destination });
    }
    Ok(outputs)
}

fn extract_source_text(source: &Path) -> Result<String> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    if extension.eq_ignore_ascii_case("docx") {
        return extract_docx_text(source);
    }

    let bytes =
        std::fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
    let mut text = decode_text_file(&bytes);
    if extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown") {
        text = markdown_to_plain_text(&text);
    }
    Ok(text)
}

fn extract_docx_text(source: &Path) -> Result<String> {
    let file =
        File::open(source).with_context(|| format!("failed to open {}", source.display()))?;
    let mut archive = ZipArchive::new(file).context("failed to read DOCX package")?;
    let document_xml = read_zip_file(&mut archive, "word/document.xml")
        .context("DOCX package does not contain word/document.xml")?;
    let text = word_document_xml_to_text(&document_xml)?;
    if text.trim().is_empty() {
        bail!("DOCX did not contain extractable text");
    }
    Ok(text)
}

fn read_zip_file<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<String> {
    if let Ok(mut file) = archive.by_name(name) {
        return read_zip_file_to_string(&mut file, name);
    }

    let normalized_target = normalize_zip_entry_name(name);
    for index in 0..archive.len() {
        let Ok(mut file) = archive.by_index(index) else {
            continue;
        };
        if normalize_zip_entry_name(file.name()) == normalized_target {
            return read_zip_file_to_string(&mut file, name);
        }
    }

    bail!("failed to read {name}: specified file not found in archive")
}

fn read_zip_file_to_string<R: Read>(file: &mut R, name: &str) -> Result<String> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to extract {name}"))?;
    String::from_utf8(bytes).context("DOCX XML is not UTF-8")
}

fn normalize_zip_entry_name(name: &str) -> String {
    name.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn word_document_xml_to_text(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.local_name().as_ref() {
                b"t" => in_text = true,
                b"tab" => text.push('\t'),
                b"br" | b"cr" => push_newline(&mut text),
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.local_name().as_ref() {
                b"tab" => text.push('\t'),
                b"br" | b"cr" => push_newline(&mut text),
                _ => {}
            },
            Ok(Event::Text(event)) if in_text => {
                text.push_str(&event.xml10_content()?.into_owned());
            }
            Ok(Event::CData(event)) if in_text => {
                text.push_str(&event.decode()?.into_owned());
            }
            Ok(Event::End(event)) => match event.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => push_newline(&mut text),
                b"tc" => {
                    if !text.ends_with('\n') && !text.ends_with('\t') {
                        text.push('\t');
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(error).context("failed to parse DOCX document XML"),
        }
    }

    Ok(collapse_blank_lines(&text))
}

fn decode_text_file(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn markdown_to_plain_text(markdown: &str) -> String {
    let mut output = String::new();
    let mut in_code_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_fence = !in_code_fence;
            continue;
        }

        if in_code_fence {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        let mut line = line.trim_start().to_owned();
        if line.starts_with('#') {
            line = line.trim_start_matches('#').trim_start().to_owned();
        }
        if line.starts_with('>') {
            line = line.trim_start_matches('>').trim_start().to_owned();
        }
        line = strip_ordered_list_marker(&line);
        line = strip_markdown_links(&line);
        line = line
            .replace("**", "")
            .replace("__", "")
            .replace('*', "")
            .replace('`', "");
        output.push_str(&line);
        output.push('\n');
    }

    collapse_blank_lines(&output)
}

fn strip_ordered_list_marker(line: &str) -> String {
    let Some((marker, rest)) = line.split_once(". ") else {
        return line.to_owned();
    };
    if marker.chars().all(|ch| ch.is_ascii_digit()) {
        rest.to_owned()
    } else {
        line.to_owned()
    }
}

fn strip_markdown_links(line: &str) -> String {
    let mut output = String::new();
    let mut rest = line;
    while let Some(label_start) = rest.find('[') {
        let before = &rest[..label_start];
        output.push_str(before);
        let after_label_start = &rest[label_start + 1..];
        let Some(label_end) = after_label_start.find(']') else {
            output.push_str(&rest[label_start..]);
            return output;
        };
        let label = &after_label_start[..label_end];
        let after_label = &after_label_start[label_end + 1..];
        if let Some(after_url_start) = after_label.strip_prefix('(') {
            if let Some(url_end) = after_url_start.find(')') {
                let url = &after_url_start[..url_end];
                output.push_str(label);
                if !url.trim().is_empty() {
                    output.push_str(" (");
                    output.push_str(url);
                    output.push(')');
                }
                rest = &after_url_start[url_end + 1..];
                continue;
            }
        }
        output.push('[');
        output.push_str(label);
        output.push(']');
        rest = after_label;
    }
    output.push_str(rest);
    output
}

fn write_text_pdf(text: &str, source: &Path, destination: &Path) -> Result<()> {
    let title = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Converted document");
    let lines = wrapped_lines(text, usable_chars_per_line());
    let lines_per_page = ((PAGE_HEIGHT - MARGIN_TOP - MARGIN_BOTTOM) / LINE_HEIGHT)
        .floor()
        .max(1.0) as usize;
    let pages = lines
        .chunks(lines_per_page)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let pages = if pages.is_empty() {
        vec![vec![String::new()]]
    } else {
        pages
    };

    let mut document = Document::with_version("1.5");
    let font_id = document.add_object(dictionary! {
        "Type" => Object::Name(b"Font".to_vec()),
        "Subtype" => Object::Name(b"Type1".to_vec()),
        "BaseFont" => Object::Name(b"Helvetica".to_vec()),
        "Encoding" => Object::Name(b"WinAnsiEncoding".to_vec()),
    });
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::with_capacity(pages.len());

    for page_lines in pages {
        let content = page_content(&page_lines)?;
        let content_id = document.add_object(Stream::new(dictionary! {}, content));
        let page_id = document.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => Object::Array(vec![0.into(), 0.into(), PAGE_WIDTH.into(), PAGE_HEIGHT.into()]),
            "Resources" => Object::Dictionary(dictionary! {
                "Font" => Object::Dictionary(dictionary! {
                    "F1" => Object::Reference(font_id)
                })
            }),
            "Contents" => Object::Reference(content_id),
        });
        page_ids.push(page_id);
    }

    let kids = page_ids
        .iter()
        .copied()
        .map(Object::Reference)
        .collect::<Vec<_>>();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Pages".to_vec()),
            "Kids" => Object::Array(kids),
            "Count" => Object::Integer(page_ids.len() as i64),
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => Object::Name(b"Catalog".to_vec()),
        "Pages" => Object::Reference(pages_id),
    });
    document.trailer.set("Root", Object::Reference(catalog_id));
    let info_id = document.add_object(dictionary! {
        "Title" => Object::String(win_ansi_bytes(title), StringFormat::Literal),
        "Creator" => Object::String(b"LawPDF".to_vec(), StringFormat::Literal),
        "Producer" => Object::String(b"LawPDF".to_vec(), StringFormat::Literal),
    });
    document.trailer.set("Info", Object::Reference(info_id));
    document.compress();
    document
        .save(destination)
        .with_context(|| format!("failed to save {}", destination.display()))?;
    Ok(())
}

fn page_content(lines: &[String]) -> Result<Vec<u8>> {
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new(
            "Tf",
            vec![Object::Name(b"F1".to_vec()), Object::Real(BODY_FONT_SIZE)],
        ),
        Operation::new("TL", vec![Object::Real(LINE_HEIGHT)]),
        Operation::new(
            "Td",
            vec![
                Object::Real(MARGIN_LEFT),
                Object::Real(PAGE_HEIGHT - MARGIN_TOP - BODY_FONT_SIZE),
            ],
        ),
    ];

    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            operations.push(Operation::new("T*", vec![]));
        }
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(win_ansi_bytes(line), StringFormat::Literal)],
        ));
    }
    operations.push(Operation::new("ET", vec![]));

    Content { operations }
        .encode()
        .context("failed to encode generated PDF page")
}

fn wrapped_lines(text: &str, chars_per_line: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.replace('\t', "    ").lines() {
        let paragraph = paragraph.trim_end();
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let word_len = word.chars().count();
            let current_len = current.chars().count();
            if current_len > 0 && current_len + 1 + word_len > chars_per_line {
                lines.push(current);
                current = String::new();
            }

            if word_len > chars_per_line {
                if !current.is_empty() {
                    lines.push(current);
                    current = String::new();
                }
                let chars = word.chars().collect::<Vec<_>>();
                for chunk in chars.chunks(chars_per_line) {
                    lines.push(chunk.iter().collect());
                }
                continue;
            }

            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

fn usable_chars_per_line() -> usize {
    let usable_width = PAGE_WIDTH - (MARGIN_LEFT * 2.0);
    (usable_width / (BODY_FONT_SIZE * 0.52)).floor().max(20.0) as usize
}

fn next_pdf_path(source: &Path) -> Result<PathBuf> {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow!("source has no usable file name: {}", source.display()))?;
    let first = parent.join(format!("{stem}.pdf"));
    if !first.exists() {
        return Ok(first);
    }

    for attempt in 1..MAX_COLLISION_ATTEMPTS {
        let candidate = parent.join(format!("{stem}-{attempt}.pdf"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "could not choose an unused PDF name beside {}",
        source.display()
    )
}

fn collapse_blank_lines(text: &str) -> String {
    let mut output = String::new();
    let mut blank_count = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                output.push('\n');
            }
        } else {
            blank_count = 0;
            output.push_str(line.trim_end());
            output.push('\n');
        }
    }
    output.trim_end().to_owned()
}

fn push_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

fn win_ansi_bytes(text: &str) -> Vec<u8> {
    text.chars().map(win_ansi_byte).collect()
}

fn win_ansi_byte(ch: char) -> u8 {
    match ch {
        '\u{20ac}' => 0x80,
        '\u{201a}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201e}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02c6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8a,
        '\u{2039}' => 0x8b,
        '\u{0152}' => 0x8c,
        '\u{017d}' => 0x8e,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02dc}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9a,
        '\u{203a}' => 0x9b,
        '\u{0153}' => 0x9c,
        '\u{017e}' => 0x9e,
        '\u{0178}' => 0x9f,
        '\u{00a0}'..='\u{00ff}' => ch as u32 as u8,
        ch if ch.is_ascii_graphic() || ch == ' ' => ch as u8,
        _ => b'?',
    }
}
