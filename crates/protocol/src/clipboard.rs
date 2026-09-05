//! Clipboard content as it travels between peers (on a stream of its own, not as a message).

use serde::{Deserialize, Serialize};

use crate::ClipFormat;

/// Largest clipboard content that is transferred at all.
pub const MAX_CLIP_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipContent {
    pub text: Option<String>,
    pub html: Option<String>,
    pub rtf: Option<Vec<u8>>,
    pub png: Option<Vec<u8>>,
}

impl ClipContent {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.rtf.is_none() && self.png.is_none()
    }

    pub fn formats(&self) -> Vec<ClipFormat> {
        let mut out = Vec::new();
        if self.text.is_some() {
            out.push(ClipFormat::Text);
        }
        if self.html.is_some() {
            out.push(ClipFormat::Html);
        }
        if self.rtf.is_some() {
            out.push(ClipFormat::Rtf);
        }
        if self.png.is_some() {
            out.push(ClipFormat::Png);
        }
        out
    }

    /// Bytes over all formats.
    pub fn size(&self) -> usize {
        self.text.as_ref().map_or(0, String::len)
            + self.html.as_ref().map_or(0, String::len)
            + self.rtf.as_ref().map_or(0, Vec::len)
            + self.png.as_ref().map_or(0, Vec::len)
    }

    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("clipboard content always encodes")
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_CLIP_LEN + 1024 {
            return None;
        }
        match postcard::take_from_bytes(bytes) {
            Ok((content, [])) => Some(content),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_garbage() {
        let content = ClipContent {
            text: Some("Grüße 🙂".into()),
            html: Some("<b>Grüße</b>".into()),
            rtf: None,
            png: Some(vec![0x89, b'P', b'N', b'G']),
        };
        assert_eq!(ClipContent::decode(&content.encode()), Some(content.clone()));
        assert_eq!(content.formats(), [ClipFormat::Text, ClipFormat::Html, ClipFormat::Png]);
        assert_eq!(content.size(), "Grüße 🙂".len() + "<b>Grüße</b>".len() + 4);
        assert!(ClipContent::decode(&[0xFF; 10]).is_none());
        assert!(ClipContent::default().is_empty());
    }
}
