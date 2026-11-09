use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_TEXT: usize = 16000;

pub fn text_decode(data: &[u8]) -> Result<String> {
    // Check for UTF-8 with optional BOM
    let slice = if data.starts_with(b"\xef\xbb\xbf") {
        &data[3..]
    } else {
        data
    };

    if let Ok(text) = std::str::from_utf8(slice) {
        if text.contains('\0') {
            bail!("binary content is not a text document");
        }
        return Ok(text.to_string());
    }

    // Try GB18030 / GBK fallback via a small decoding loop or heuristic
    if let Some(text) = decode_gb18030(slice) {
        if text.contains('\0') {
            bail!("binary content is not a text document");
        }
        return Ok(text);
    }

    bail!("unsupported text encoding");
}

fn decode_gb18030(data: &[u8]) -> Option<String> {
    unsafe extern "C" {
        fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
        fn iconv(
            cd: *mut c_void,
            inbuf: *mut *const c_char,
            inbytesleft: *mut usize,
            outbuf: *mut *mut c_char,
            outbytesleft: *mut usize,
        ) -> usize;
        fn iconv_close(cd: *mut c_void) -> c_int;
    }

    unsafe {
        let to = b"UTF-8\0".as_ptr() as *const c_char;
        let from = b"GB18030\0".as_ptr() as *const c_char;
        let cd = iconv_open(to, from);
        if cd == usize::MAX as *mut c_void || cd.is_null() {
            return None;
        }

        let mut in_bytes_left = data.len();
        let mut in_ptr = data.as_ptr() as *const c_char;

        let mut out = vec![0u8; data.len() * 4 + 1];
        let mut out_bytes_left = out.len();
        let mut out_ptr = out.as_mut_ptr() as *mut c_char;

        let res = iconv(
            cd,
            &mut in_ptr,
            &mut in_bytes_left,
            &mut out_ptr,
            &mut out_bytes_left,
        );
        iconv_close(cd);

        if res == usize::MAX {
            None
        } else {
            let written = out.len() - out_bytes_left;
            out.truncate(written);
            String::from_utf8(out).ok()
        }
    }
}

pub fn parse_csv_simple(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => {
                    current_row.push(current_field);
                    current_field = String::new();
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    current_row.push(current_field);
                    current_field = String::new();
                    rows.push(current_row);
                    current_row = Vec::new();
                }
                '\n' => {
                    current_row.push(current_field);
                    current_field = String::new();
                    rows.push(current_row);
                    current_row = Vec::new();
                }
                _ => current_field.push(c),
            }
        }
    }

    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field);
        rows.push(current_row);
    }

    rows
}

struct ZipEntryInfo {
    filename: String,
    compression: u16,
    compressed_size: u32,
    uncompressed_size: u32,
    local_offset: u32,
}

struct ZipArchive {
    data: Vec<u8>,
    entries: Vec<ZipEntryInfo>,
}

impl ZipArchive {
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        if data.len() < 22 {
            bail!("invalid zip archive");
        }

        // Search for EOCD from end
        let mut eocd_pos = None;
        let search_start = if data.len() > 65557 {
            data.len() - 65557
        } else {
            0
        };

        for i in (search_start..=data.len() - 22).rev() {
            if &data[i..i + 4] == b"PK\x05\x06" {
                eocd_pos = Some(i);
                break;
            }
        }

        let eocd_idx = eocd_pos.context("not a valid zip file")?;
        let entry_count =
            u16::from_le_bytes(data[eocd_idx + 10..eocd_idx + 12].try_into()?) as usize;
        let _cd_size = u32::from_le_bytes(data[eocd_idx + 12..eocd_idx + 16].try_into()?) as usize;
        let cd_offset = u32::from_le_bytes(data[eocd_idx + 16..eocd_idx + 20].try_into()?) as usize;

        if cd_offset >= data.len() {
            bail!("invalid zip central directory offset");
        }

        let mut offset = cd_offset;
        let mut entries = Vec::with_capacity(entry_count);
        let mut total_uncompressed: u64 = 0;

        for _ in 0..entry_count {
            if offset + 46 > data.len() {
                bail!("corrupt zip entry header");
            }
            if &data[offset..offset + 4] != b"PK\x01\x02" {
                bail!("corrupt zip central directory signature");
            }

            let flag_bits = u16::from_le_bytes(data[offset + 8..offset + 10].try_into()?);
            let compression = u16::from_le_bytes(data[offset + 10..offset + 12].try_into()?);
            let compressed_size = u32::from_le_bytes(data[offset + 20..offset + 24].try_into()?);
            let uncompressed_size = u32::from_le_bytes(data[offset + 24..offset + 28].try_into()?);
            let name_len = u16::from_le_bytes(data[offset + 28..offset + 30].try_into()?) as usize;
            let extra_len = u16::from_le_bytes(data[offset + 30..offset + 32].try_into()?) as usize;
            let comment_len =
                u16::from_le_bytes(data[offset + 32..offset + 34].try_into()?) as usize;
            let local_offset = u32::from_le_bytes(data[offset + 42..offset + 46].try_into()?);

            if offset + 46 + name_len > data.len() {
                bail!("corrupt zip filename");
            }

            let name_bytes = &data[offset + 46..offset + 46 + name_len];
            let filename = String::from_utf8_lossy(name_bytes).to_string();

            if (flag_bits & 1) != 0 {
                bail!("encrypted document unsupported");
            }

            total_uncompressed += uncompressed_size as u64;

            entries.push(ZipEntryInfo {
                filename,
                compression,
                compressed_size,
                uncompressed_size,
                local_offset,
            });

            offset += 46 + name_len + extra_len + comment_len;
        }

        if entries.len() > 10000 || total_uncompressed > 20 * 1024 * 1024 {
            bail!("expanded document size limit");
        }

        Ok(Self { data, entries })
    }

    fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.filename == name)
            .ok_or_else(|| anyhow::anyhow!("entry not found in zip: {}", name))?;

        let offset = entry.local_offset as usize;
        if offset + 30 > self.data.len() {
            bail!("invalid local file header offset");
        }

        if &self.data[offset..offset + 4] != b"PK\x03\x04" {
            bail!("invalid local file header signature");
        }

        let name_len = u16::from_le_bytes(self.data[offset + 26..offset + 28].try_into()?) as usize;
        let extra_len =
            u16::from_le_bytes(self.data[offset + 28..offset + 30].try_into()?) as usize;

        let data_start = offset + 30 + name_len + extra_len;
        let data_end = data_start + entry.compressed_size as usize;

        if data_end > self.data.len() {
            bail!("unexpected end of zip data");
        }

        let compressed = &self.data[data_start..data_end];

        if entry.compression == 0 {
            // Stored
            Ok(compressed.to_vec())
        } else if entry.compression == 8 {
            // Raw Deflate
            decompress_raw_deflate(compressed, entry.uncompressed_size as usize)
        } else {
            bail!("unsupported zip compression method: {}", entry.compression);
        }
    }
}

fn decompress_raw_deflate(_compressed: &[u8], _expected_size: usize) -> Result<Vec<u8>> {
    bail!("compressed zip entry requires uncompressed storage or miniz");
}

fn read_xml_part(archive: &ZipArchive, name: &str) -> Result<String> {
    let data = archive.read_entry(name)?;
    if data.len() > 2 * 1024 * 1024 {
        bail!("XML part too large");
    }

    let upper = data.to_ascii_uppercase();
    if upper.windows(9).any(|w| w == b"<!DOCTYPE") || upper.windows(8).any(|w| w == b"<!ENTITY") {
        bail!("document XML entities are not accepted");
    }

    String::from_utf8(data).context("invalid XML utf-8 encoding")
}

pub fn extract(
    path: &Path,
    extension: &str,
    remote: Option<&Value>,
    max_bytes: Option<u64>,
) -> Result<Value> {
    let limit = max_bytes.unwrap_or(MAX_BYTES);
    if !path.is_file() {
        bail!("file size limit");
    }
    let file_size = path.metadata()?.len();
    if file_size > limit {
        bail!("file size limit");
    }

    let ext = extension.to_lowercase();
    if ext == "pdf" {
        let mut header = [0u8; 5];
        let mut f = File::open(path)?;
        let n = f.read(&mut header)?;
        if n < 5 || &header != b"%PDF-" || remote.is_none() || !remote.unwrap().is_object() {
            bail!("PDF extraction unavailable");
        }
        let rem = remote.unwrap();
        let full_text = rem.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let truncated_chars: String = full_text.chars().take(MAX_TEXT).collect();
        return Ok(json!({
            "text": truncated_chars,
            "page_count": rem.get("page_count"),
            "truncated": rem.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false),
            "scanned_or_empty": rem.get("scanned_or_empty").and_then(|v| v.as_bool()).unwrap_or(false)
        }));
    }

    if ext == "txt" || ext == "md" || ext == "csv" || ext == "json" {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        let text = text_decode(&bytes)?;

        if ext == "json" {
            let parsed: Value = serde_json::from_str(&text).context("invalid json")?;
            let formatted = serde_json::to_string_pretty(&parsed)?;
            let count = formatted.chars().count();
            let truncated_str: String = formatted.chars().take(MAX_TEXT).collect();
            return Ok(json!({
                "text": truncated_str,
                "truncated": count > MAX_TEXT
            }));
        }

        if ext == "csv" {
            let all_rows = parse_csv_simple(&text);
            let total_rows = all_rows.len();
            let mut rows = Vec::new();
            for r in all_rows.into_iter().take(100) {
                let row_truncated: Vec<String> = r.into_iter().take(30).collect();
                rows.push(row_truncated);
            }
            let truncated = if text.is_empty() {
                false
            } else {
                total_rows > 100
            };
            return Ok(json!({
                "rows": rows,
                "truncated": truncated
            }));
        }

        let count = text.chars().count();
        let truncated_str: String = text.chars().take(MAX_TEXT).collect();
        return Ok(json!({
            "text": truncated_str,
            "truncated": count > MAX_TEXT
        }));
    }

    if ext != "docx" && ext != "xlsx" {
        bail!("unsupported document type");
    }

    let archive = ZipArchive::open(path)?;

    if ext == "docx" {
        let xml_str = read_xml_part(&archive, "word/document.xml")?;
        // Extract paragraphs <w:p> and text <w:t>
        let paragraphs = extract_docx_paragraphs(&xml_str);
        let joined = paragraphs.join("\n");
        let count = joined.chars().count();
        let truncated_str: String = joined.chars().take(MAX_TEXT).collect();
        return Ok(json!({
            "text": truncated_str,
            "truncated": count > MAX_TEXT,
            "embedded_images_read": false
        }));
    }

    // xlsx
    let shared_strings = if archive
        .entries
        .iter()
        .any(|e| e.filename == "xl/sharedStrings.xml")
    {
        let ss_xml = read_xml_part(&archive, "xl/sharedStrings.xml")?;
        extract_shared_strings(&ss_xml)
    } else {
        Vec::new()
    };

    let rels_xml = read_xml_part(&archive, "xl/_rels/workbook.xml.rels")?;
    let targets = extract_relationships(&rels_xml);

    let wb_xml = read_xml_part(&archive, "xl/workbook.xml")?;
    let (sheets_meta, total_sheets) = extract_workbook_sheets(&wb_xml);

    let mut sheets = Vec::new();
    for (sheet_name, rel_id) in sheets_meta.into_iter().take(3) {
        let target = targets
            .get(&rel_id)
            .context("relationship target not found")?;
        let normalized_target = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else {
            format!("xl/{}", target)
        };

        if !normalized_target.starts_with("xl/") || normalized_target.contains("/..") {
            bail!("invalid worksheet target");
        }

        let sheet_xml = read_xml_part(&archive, &normalized_target)?;
        let (rows, total_rows) = extract_sheet_rows(&sheet_xml, &shared_strings);

        sheets.push(json!({
            "name": sheet_name,
            "rows": rows,
            "truncated": total_rows > 100
        }));
    }

    Ok(json!({
        "sheets": sheets,
        "truncated": total_sheets > 3,
        "formulas_executed": false
    }))
}

fn extract_docx_paragraphs(xml: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut rest = xml;

    while let Some(p_start) = rest.find("<w:p") {
        let after_p = &rest[p_start..];
        let p_end = match after_p.find("</w:p>") {
            Some(idx) => idx + 6,
            None => break,
        };
        let p_content = &after_p[..p_end];
        rest = &after_p[p_end..];

        let mut p_text = String::new();
        let mut cur = p_content;
        while let Some(t_start) = cur.find("<w:t") {
            let after_t = &cur[t_start..];
            let tag_close = match after_t.find('>') {
                Some(idx) => idx + 1,
                None => break,
            };
            let text_start = &after_t[tag_close..];
            let t_end = match text_start.find("</w:t>") {
                Some(idx) => idx,
                None => break,
            };
            p_text.push_str(&text_start[..t_end]);
            cur = &text_start[t_end + 6..];
        }

        paragraphs.push(p_text);
    }

    paragraphs
}

fn extract_shared_strings(xml: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut rest = xml;

    while let Some(si_start) = rest.find("<s:si") {
        let after_si = &rest[si_start..];
        let si_end = match after_si.find("</s:si>") {
            Some(idx) => idx + 7,
            None => break,
        };
        let si_content = &after_si[..si_end];
        rest = &after_si[si_end..];

        let mut si_text = String::new();
        let mut cur = si_content;
        while let Some(t_start) = cur.find("<s:t") {
            let after_t = &cur[t_start..];
            let tag_close = match after_t.find('>') {
                Some(idx) => idx + 1,
                None => break,
            };
            let text_start = &after_t[tag_close..];
            let t_end = match text_start.find("</s:t>") {
                Some(idx) => idx,
                None => break,
            };
            si_text.push_str(&text_start[..t_end]);
            cur = &text_start[t_end + 6..];
        }

        strings.push(si_text);
    }

    strings
}

fn extract_relationships(xml: &str) -> HashMap<String, String> {
    let mut targets = HashMap::new();
    let mut rest = xml;

    while let Some(rel_start) = rest.find("<Relationship") {
        let after = &rest[rel_start..];
        let rel_end = match after.find('>') {
            Some(idx) => idx + 1,
            None => break,
        };
        let rel_tag = &after[..rel_end];
        rest = &after[rel_end..];

        let id = extract_attr(rel_tag, "Id");
        let target = extract_attr(rel_tag, "Target");
        let target_mode = extract_attr(rel_tag, "TargetMode");

        if target_mode.as_deref() != Some("External") {
            if let (Some(i), Some(t)) = (id, target) {
                targets.insert(i, t);
            }
        }
    }

    targets
}

fn extract_workbook_sheets(xml: &str) -> (Vec<(String, String)>, usize) {
    let mut sheets = Vec::new();
    let mut rest = xml;
    let mut total = 0;

    while let Some(sheet_start) = rest.find("<s:sheet") {
        let after = &rest[sheet_start..];
        let sheet_end = match after.find('>') {
            Some(idx) => idx + 1,
            None => break,
        };
        let sheet_tag = &after[..sheet_end];
        rest = &after[sheet_end..];

        total += 1;
        let name = extract_attr(sheet_tag, "name").unwrap_or_default();
        let rid = extract_attr(sheet_tag, "r:id").unwrap_or_default();
        sheets.push((name, rid));
    }

    (sheets, total)
}

fn extract_sheet_rows(xml: &str, shared_strings: &[String]) -> (Vec<Vec<Value>>, usize) {
    let mut rows = Vec::new();
    let mut rest = xml;
    let mut total_rows = 0;

    while let Some(row_start) = rest.find("<s:row") {
        let after_row = &rest[row_start..];
        let row_end = match after_row.find("</s:row>") {
            Some(idx) => idx + 8,
            None => break,
        };
        let row_content = &after_row[..row_end];
        rest = &after_row[row_end..];

        total_rows += 1;
        if rows.len() >= 100 {
            continue;
        }

        let mut cells = Vec::new();
        let mut cur = row_content;

        while let Some(c_start) = cur.find("<s:c") {
            let after_c = &cur[c_start..];
            let tag_close = match after_c.find('>') {
                Some(idx) => idx + 1,
                None => break,
            };
            let c_tag = &after_c[..tag_close];

            let c_end = if c_tag.ends_with("/>") {
                tag_close
            } else {
                match after_c.find("</s:c>") {
                    Some(idx) => idx + 6,
                    None => break,
                }
            };

            let c_content = &after_c[..c_end];
            cur = &after_c[c_end..];

            if cells.len() >= 30 {
                continue;
            }

            let r_attr = extract_attr(c_tag, "r");
            let t_attr = extract_attr(c_tag, "t");

            let val = if t_attr.as_deref() == Some("s") {
                let v = extract_tag_text(c_content, "<s:v>", "</s:v>").unwrap_or_default();
                if let Ok(idx) = v.parse::<usize>() {
                    shared_strings.get(idx).cloned().unwrap_or_default()
                } else {
                    v
                }
            } else if t_attr.as_deref() == Some("inlineStr") {
                extract_tag_text(c_content, "<s:t>", "</s:t>").unwrap_or_default()
            } else {
                let v = extract_tag_text(c_content, "<s:v>", "</s:v>");
                let has_f = c_content.contains("<s:f>") || c_content.contains("<s:f ");
                if has_f && v.as_deref().unwrap_or("").is_empty() {
                    "[公式无缓存值，未执行]".to_string()
                } else {
                    v.unwrap_or_default()
                }
            };

            cells.push(json!({
                "cell": r_attr,
                "value": val
            }));
        }

        rows.push(cells);
    }

    (rows, total_rows)
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn extract_tag_text(content: &str, open: &str, close: &str) -> Option<String> {
    let start = content.find(open)? + open.len();
    let end = content[start..].find(close)? + start;
    Some(content[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            let p = std::env::temp_dir().join(format!("{}-{}", prefix, uuid::Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&p);
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_simple_zip(path: &Path, files: &[(&str, &[u8], bool)]) {
        let mut central_dir = Vec::new();
        let mut body = Vec::new();

        for (name, data, _) in files {
            let local_offset = body.len() as u32;

            // Local file header (compression=0)
            body.extend_from_slice(b"PK\x03\x04");
            body.extend_from_slice(&20u16.to_le_bytes()); // version
            body.extend_from_slice(&0u16.to_le_bytes()); // flag
            body.extend_from_slice(&0u16.to_le_bytes()); // compression=0
            body.extend_from_slice(&0u32.to_le_bytes()); // mod time
            body.extend_from_slice(&0u32.to_le_bytes()); // crc32
            body.extend_from_slice(&(data.len() as u32).to_le_bytes()); // comp size
            body.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncomp size
            body.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            body.extend_from_slice(&0u16.to_le_bytes()); // extra len
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(data);

            // Central directory entry
            central_dir.extend_from_slice(b"PK\x01\x02");
            central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // flag
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // compression=0
            central_dir.extend_from_slice(&0u32.to_le_bytes()); // mod time
            central_dir.extend_from_slice(&0u32.to_le_bytes()); // crc32
            central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes()); // comp size
            central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncomp size
            central_dir.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // extra len
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // comment len
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // disk start
            central_dir.extend_from_slice(&0u16.to_le_bytes()); // internal attr
            central_dir.extend_from_slice(&0u32.to_le_bytes()); // external attr
            central_dir.extend_from_slice(&local_offset.to_le_bytes());
            central_dir.extend_from_slice(name.as_bytes());
        }

        let cd_offset = body.len() as u32;
        let cd_size = central_dir.len() as u32;
        let entry_count = files.len() as u16;

        body.extend_from_slice(&central_dir);

        // EOCD
        body.extend_from_slice(b"PK\x05\x06");
        body.extend_from_slice(&0u16.to_le_bytes()); // disk
        body.extend_from_slice(&0u16.to_le_bytes()); // start disk
        body.extend_from_slice(&entry_count.to_le_bytes());
        body.extend_from_slice(&entry_count.to_le_bytes());
        body.extend_from_slice(&cd_size.to_le_bytes());
        body.extend_from_slice(&cd_offset.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes()); // comment len

        fs::write(path, body).unwrap();
    }

    #[test]
    fn test_bot_documents_text_and_csv_contents_are_actually_read() {
        let temp = TempDir::new("doc-test-1");
        let p_txt = temp.path().join("a.txt");
        fs::write(&p_txt, "口令：竹叶-47").unwrap();
        let res_txt = extract(&p_txt, "txt", None, None).unwrap();
        assert!(res_txt["text"].as_str().unwrap().contains("竹叶-47"));

        let p_csv = temp.path().join("a.csv");
        fs::write(&p_csv, "名称,数量\n苹果,3\n").unwrap();
        let res_csv = extract(&p_csv, "csv", None, None).unwrap();
        assert_eq!(res_csv["rows"][1], json!(["苹果", "3"]));
    }

    #[test]
    fn test_bot_documents_binary_disguised_as_text_rejected() {
        let temp = TempDir::new("doc-test-2");
        let p = temp.path().join("a.txt");
        fs::write(&p, b"bad\x00binary").unwrap();
        assert!(extract(&p, "txt", None, None).is_err());
    }

    #[test]
    fn test_bot_documents_docx_reads_document_xml() {
        let temp = TempDir::new("doc-test-3");
        let p = temp.path().join("a.docx");
        let xml = "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>实际正文 竹叶-47</w:t></w:r></w:p></w:body></w:document>";
        write_simple_zip(&p, &[("word/document.xml", xml.as_bytes(), false)]);

        let res = extract(&p, "docx", None, None).unwrap();
        assert!(res["text"].as_str().unwrap().contains("实际正文 竹叶-47"));
    }

    #[test]
    fn test_bot_documents_xml_entities_rejected() {
        let temp = TempDir::new("doc-test-4");
        let p = temp.path().join("a.docx");
        let xml = "<!DOCTYPE x [<!ENTITY e \"bad\">]><x>&e;</x>";
        write_simple_zip(&p, &[("word/document.xml", xml.as_bytes(), false)]);
        assert!(extract(&p, "docx", None, None).is_err());
    }

    #[test]
    fn test_bot_documents_xlsx_reads_cells_without_executing_formulas() {
        let temp = TempDir::new("doc-test-5");
        let p = temp.path().join("a.xlsx");

        let wb_xml = "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><s:sheet name=\"Sheet1\" r:id=\"r1\"/></sheets></workbook>";
        let rels_xml = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"r1\" Target=\"worksheets/sheet1.xml\"/></Relationships>";
        let sheet1_xml = "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><s:sheetData><s:row><s:c r=\"A1\" t=\"inlineStr\"><s:is><s:t>竹叶-47</s:t></s:is></s:c><s:c r=\"B1\"><s:f>HYPERLINK(\"https://example.invalid\")</s:f></s:c></s:row></s:sheetData></worksheet>";

        write_simple_zip(
            &p,
            &[
                ("xl/workbook.xml", wb_xml.as_bytes(), false),
                ("xl/_rels/workbook.xml.rels", rels_xml.as_bytes(), false),
                ("xl/worksheets/sheet1.xml", sheet1_xml.as_bytes(), false),
            ],
        );

        let res = extract(&p, "xlsx", None, None).unwrap();
        assert_eq!(res["sheets"][0]["rows"][0][0]["value"], "竹叶-47");
        assert_eq!(res["formulas_executed"], false);
    }

    #[test]
    fn test_bot_documents_pdf_requires_original_header_and_actual_extraction() {
        let temp = TempDir::new("doc-test-6");
        let p = temp.path().join("a.pdf");
        fs::write(&p, b"%PDF-1.7\nfixture").unwrap();

        assert!(extract(&p, "pdf", None, None).is_err());

        let remote = json!({
            "text": "真实提取",
            "page_count": 1
        });
        let res = extract(&p, "pdf", Some(&remote), None).unwrap();
        assert_eq!(res["text"], "真实提取");
    }

    #[test]
    fn test_bot_documents_oversized_zip_expansion_rejected() {
        let temp = TempDir::new("doc-test-7");
        let p = temp.path().join("a.docx");

        // Manually write zip with uncompressed_size = 21 * 1024 * 1024 in CD
        let name = "word/document.xml";
        let uncompressed_size = 21 * 1024 * 1024u32;
        let mut central_dir = Vec::new();
        let mut body = Vec::new();

        body.extend_from_slice(b"PK\x03\x04");
        body.extend_from_slice(&20u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&8u16.to_le_bytes()); // deflated
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&10u32.to_le_bytes()); // comp size
        body.extend_from_slice(&uncompressed_size.to_le_bytes());
        body.extend_from_slice(&(name.len() as u16).to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(&[0u8; 10]);

        central_dir.extend_from_slice(b"PK\x01\x02");
        central_dir.extend_from_slice(&20u16.to_le_bytes());
        central_dir.extend_from_slice(&20u16.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&8u16.to_le_bytes());
        central_dir.extend_from_slice(&0u32.to_le_bytes());
        central_dir.extend_from_slice(&0u32.to_le_bytes());
        central_dir.extend_from_slice(&10u32.to_le_bytes());
        central_dir.extend_from_slice(&uncompressed_size.to_le_bytes());
        central_dir.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes());
        central_dir.extend_from_slice(&0u32.to_le_bytes());
        central_dir.extend_from_slice(&0u32.to_le_bytes()); // local offset 0
        central_dir.extend_from_slice(name.as_bytes());

        let cd_offset = body.len() as u32;
        let cd_size = central_dir.len() as u32;
        body.extend_from_slice(&central_dir);

        body.extend_from_slice(b"PK\x05\x06");
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&cd_size.to_le_bytes());
        body.extend_from_slice(&cd_offset.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());

        fs::write(&p, body).unwrap();

        let err = extract(&p, "docx", None, None).unwrap_err();
        assert!(err.to_string().contains("expanded"));
    }

    #[test]
    fn test_attachment_limit_documents_extract_follows_max_bytes_parameter() {
        assert_eq!(MAX_BYTES, 50 * 1024 * 1024);
        let temp = TempDir::new("doc-test-8");
        let p = temp.path().join("sample.txt");
        fs::write(&p, b"hello world").unwrap();

        let res = extract(&p, "txt", None, None).unwrap();
        assert_eq!(res["text"], "hello world");

        let err = extract(&p, "txt", None, Some(5)).unwrap_err();
        assert!(err.to_string().contains("file size limit"));

        let res_high = extract(&p, "txt", None, Some(100)).unwrap();
        assert_eq!(res_high["text"], "hello world");
    }
}
