//! A transcript as a Word document (.docx) on the Desktop: a title and one paragraph per piece of speech.
//! Arabic paragraphs are right-to-left. Written by hand (a .docx is a zip of three XML files).

use std::io::Write;
use std::path::{Path, PathBuf};

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            c => c.to_string(),
        })
        .collect()
}

fn paragraph(text: &str, title: bool) -> String {
    let rtl = text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    let ppr = if rtl { r#"<w:pPr><w:bidi/></w:pPr>"# } else { "" };
    let rpr = if title { r#"<w:rPr><w:b/><w:bCs/><w:sz w:val="36"/><w:szCs w:val="36"/></w:rPr>"# } else { "" };
    format!(r#"<w:p>{ppr}<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, esc(text))
}

pub fn write(path: &Path, title: &str, paragraphs: &[String]) -> Result<(), String> {
    let body: String = std::iter::once(paragraph(title, true)).chain(paragraphs.iter().map(|p| paragraph(p, false))).collect();
    let files = [
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
        ("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#)),
    ];
    let out = std::fs::File::create(path).map_err(|e| format!("Couldn't save the document: {e}"))?;
    let mut zip = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, xml) in files {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// The user's Desktop (where Windows really keeps it, e.g. inside OneDrive).
pub fn desktop() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KNOWN_FOLDER_FLAG(0), None) {
            let path = p.to_string().map(PathBuf::from);
            CoTaskMemFree(Some(p.0 as _));
            if let Ok(path) = path {
                return path;
            }
        }
    }
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Desktop")
}

/// "<title>.docx" in `dir`, made safe for a file name, never overwriting ("… (2).docx").
pub fn free_name(dir: &Path, title: &str) -> PathBuf {
    let clean: String = title.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c) && !c.is_control()).take(80).collect();
    let clean = clean.trim().trim_end_matches('.').trim();
    let base = if clean.is_empty() { "Transcript" } else { clean };
    (1..).map(|n| dir.join(if n == 1 { format!("{base}.docx") } else { format!("{base} ({n}).docx") }))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_docx_with_a_safe_unique_name() {
        let dir = std::env::temp_dir().join(format!("nabra-docx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = free_name(&dir, "What: AI <is> next?");
        assert_eq!(a.file_name().unwrap(), "What AI is next.docx");
        write(&a, "What AI is next", &["Hello & welcome".into(), "مرحبا بكم".into()]).unwrap();
        assert_eq!(free_name(&dir, "What: AI <is> next?").file_name().unwrap(), "What AI is next (2).docx");
        let mut z = zip::ZipArchive::new(std::fs::File::open(&a).unwrap()).unwrap();
        let mut doc = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut doc).unwrap();
        assert!(doc.contains("Hello &amp; welcome") && doc.contains("<w:bidi/>"));
        assert_eq!(free_name(&dir, "  ").file_name().unwrap(), "Transcript.docx");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
