//! The Windows clipboard's HTML format: a header with byte offsets in front of the document.

const MARK_START: &str = "<!--StartFragment-->";
const MARK_END: &str = "<!--EndFragment-->";

fn header(start_html: usize, end_html: usize, start_fragment: usize, end_fragment: usize) -> String {
    format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
         StartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    )
}

/// Wraps an HTML fragment (or document) for the Windows clipboard.
pub fn encode(html: &str) -> Vec<u8> {
    let prefix = format!("<html><body>\r\n{MARK_START}");
    let suffix = format!("{MARK_END}\r\n</body></html>");
    let header_len = header(0, 0, 0, 0).len();
    let start_fragment = header_len + prefix.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + suffix.len();
    let mut out = header(header_len, end_html, start_fragment, end_fragment).into_bytes();
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(html.as_bytes());
    out.extend_from_slice(suffix.as_bytes());
    out
}

fn offset(text: &str, key: &str) -> Option<usize> {
    let start = text.find(key)? + key.len();
    let digits: String = text[start..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The fragment from the Windows clipboard's HTML format.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.split(|b| *b == 0).next().unwrap_or(bytes);
    // The header is ASCII, so cutting it at the first '<' is safe
    let head_end = bytes.iter().position(|b| *b == b'<').unwrap_or(bytes.len());
    let head = std::str::from_utf8(&bytes[..head_end]).ok()?;
    let range = match (offset(head, "StartFragment:"), offset(head, "EndFragment:")) {
        (Some(s), Some(e)) if s <= e && e <= bytes.len() => s..e,
        _ => match (offset(head, "StartHTML:"), offset(head, "EndHTML:")) {
            (Some(s), Some(e)) if s <= e && e <= bytes.len() => s..e,
            _ => head_end..bytes.len(),
        },
    };
    Some(String::from_utf8_lossy(&bytes[range]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_umlauts() {
        let html = "<b>Grüße</b> & <i>mehr</i>";
        let encoded = encode(html);
        assert!(encoded.starts_with(b"Version:0.9\r\nStartHTML:"));
        assert_eq!(decode(&encoded).as_deref(), Some(html));
    }

    #[test]
    fn as_written_by_a_browser() {
        let doc = "<html><body>\r\n<!--StartFragment--><p>Hallo</p><!--EndFragment-->\r\n</body></html>";
        let head_len = header(0, 0, 0, 0).len();
        let start = head_len + doc.find("<p>").unwrap();
        let end = head_len + doc.find("<!--EndF").unwrap();
        let mut bytes = header(head_len, head_len + doc.len(), start, end).into_bytes();
        bytes.extend_from_slice(doc.as_bytes());
        bytes.push(0);
        assert_eq!(decode(&bytes).as_deref(), Some("<p>Hallo</p>"));
    }

    #[test]
    fn broken_offsets_fall_back() {
        let bytes = b"Version:0.9\r\nStartFragment:0000099999\r\nEndFragment:0000000001\r\n<p>x</p>";
        assert_eq!(decode(bytes).as_deref(), Some("<p>x</p>"));
        assert_eq!(decode(b"").as_deref(), Some(""));
    }
}
