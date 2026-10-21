//! Keeping the clipboards of the connected computers the same.
//!
//! A change is announced to every peer right away (formats and size). Content up to 1 MB follows
//! at once; larger content is fetched only when the user arrives at a computer that needs it.
//! Content written here on behalf of a peer is recognised by the change count and a hash, so it is
//! never announced back (no ping-pong).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tc_clipboard::{Read, SystemClipboard};
use tc_net::{Fingerprint, Node};
use tc_protocol::{ClipContent, Message};

/// Content up to this size is sent along with the announcement.
pub const EAGER_LIMIT: usize = 1024 * 1024;
const HEADER: &[u8; 4] = b"clip";

/// The system clipboard and how much of it may travel.
#[derive(Clone)]
pub struct Clipboard {
    pub system: Arc<dyn SystemClipboard>,
    /// Larger content stays on this computer.
    pub max_size: usize,
}

/// What the clipboard logic wants done.
#[derive(Debug, PartialEq)]
pub enum ClipAction {
    /// Read the local clipboard (it changed).
    Read,
    /// Write this content into the local clipboard.
    Write { id: u64, content: ClipContent },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// A password manager marked the content.
    Concealed,
    TooLarge(usize),
}

/// An announcement for the peers, with the content if it is small enough to go along.
pub type Announcement = (Message, Option<Arc<Vec<u8>>>);

fn hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

fn header(id: u64) -> Vec<u8> {
    let mut h = HEADER.to_vec();
    h.extend_from_slice(&id.to_le_bytes());
    h
}

pub fn parse_header(header: &[u8]) -> Option<u64> {
    let id = header.strip_prefix(HEADER)?;
    Some(u64::from_le_bytes(id.try_into().ok()?))
}

/// What makes content the same for a person: the text (line ends aside), else the image.
fn gist(content: &ClipContent) -> Option<u64> {
    if let Some(text) = &content.text {
        return Some(hash(text.replace("\r\n", "\n").trim_end().as_bytes()));
    }
    content.png.as_deref().map(hash)
}

#[derive(Default)]
pub struct ClipSync {
    /// Change count when we last looked (or after our own write).
    seen: Option<u64>,
    /// A read or write is running; no polling meanwhile.
    busy: bool,
    /// Our newest content, encoded, if the newest content anywhere is ours.
    local: Option<(u64, Arc<Vec<u8>>, usize)>,
    /// The newest content is on a peer.
    remote: Option<(Fingerprint, u64, u64)>,
    /// Content that came from a peer (by hash), never announced again.
    written_hash: Option<u64>,
    /// The gist of the newest content sent or received. Other clipboard syncs (Apple's Universal
    /// Clipboard, Windows' clipboard history in the cloud) bring the same content back in other
    /// formats; it is not sent again.
    last_gist: Option<u64>,
    requested: Option<u64>,
    /// Content that came before its announcement (they travel on different streams).
    early: Option<(Fingerprint, u64, Vec<u8>)>,
    counter: u64,
}

impl ClipSync {
    fn new_id(&mut self) -> u64 {
        self.counter += 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        nanos ^ self.counter.rotate_left(48)
    }

    /// Call regularly with the current change count.
    pub fn poll(&mut self, count: u64) -> Option<ClipAction> {
        if self.busy {
            return None;
        }
        match self.seen {
            Some(seen) if seen == count => None,
            None => {
                // At start the current content is not announced; only changes are
                self.seen = Some(count);
                None
            }
            Some(_) => {
                self.seen = Some(count);
                self.busy = true;
                Some(ClipAction::Read)
            }
        }
    }

    /// The local clipboard was read after a change. Returns what to tell the peers.
    pub fn read_done(&mut self, read: Read, max_size: usize) -> (Option<Announcement>, Option<Skip>) {
        self.busy = false;
        let content = match read {
            Read::Content(c) => c,
            Read::Empty => return (None, None),
            Read::Concealed => return (None, Some(Skip::Concealed)),
            Read::Files(_) => return (None, None),
        };
        let size = content.size();
        if size > max_size {
            return (None, Some(Skip::TooLarge(size)));
        }
        let encoded = content.encode();
        if Some(hash(&encoded)) == self.written_hash {
            return (None, None); // What we just wrote for a peer
        }
        let gist = gist(&content);
        if gist.is_some() && gist == self.last_gist {
            return (None, None); // The same again, from another sync
        }
        self.last_gist = gist;
        let id = self.new_id();
        let data = Arc::new(encoded);
        self.local = Some((id, data.clone(), size));
        self.remote = None;
        let offer = Message::ClipboardOffer {
            id,
            formats: content.formats(),
            size: size as u64,
        };
        let eager = (size <= EAGER_LIMIT).then_some(data);
        (Some((offer, eager)), None)
    }

    pub fn read_failed(&mut self) {
        self.busy = false;
    }

    /// Files were copied: they are the newest content now (handled by the file transfers).
    pub fn files_copied(&mut self) {
        self.busy = false;
        self.local = None;
        self.remote = None;
    }

    /// A peer announced files: its earlier clipboard content is no longer the newest.
    pub fn clear_remote(&mut self) {
        self.remote = None;
    }

    /// What a peer that just connected should get.
    pub fn for_new_peer(&self) -> Option<Announcement> {
        let (id, data, size) = self.local.as_ref()?;
        let content = ClipContent::decode(data)?;
        let offer = Message::ClipboardOffer {
            id: *id,
            formats: content.formats(),
            size: *size as u64,
        };
        Some((offer, (*size <= EAGER_LIMIT).then(|| data.clone())))
    }

    /// A peer announced new content. Returns a request to send if this computer is active and the
    /// content will not come by itself.
    pub fn offered(&mut self, from: Fingerprint, id: u64, size: u64, active: bool) -> Option<Message> {
        self.local = None;
        self.remote = Some((from, id, size));
        self.requested = None;
        (active && size as usize > EAGER_LIMIT)
            .then(|| self.request())
            .flatten()
    }

    /// This computer became the one in use: fetch pending content.
    pub fn became_active(&mut self) -> Option<(Fingerprint, Message)> {
        let (from, _, size) = self.remote?;
        if size as usize <= EAGER_LIMIT {
            return None;
        }
        self.request().map(|m| (from, m))
    }

    fn request(&mut self) -> Option<Message> {
        let (_, id, _) = self.remote?;
        if self.requested == Some(id) {
            return None;
        }
        self.requested = Some(id);
        Some(Message::ClipboardRequest { id })
    }

    /// A peer asks for our content.
    pub fn requested(&self, id: u64) -> Option<(Vec<u8>, Arc<Vec<u8>>)> {
        match &self.local {
            Some((ours, data, _)) if *ours == id => Some((header(id), data.clone())),
            _ => None,
        }
    }

    /// Content that arrived before its announcement, now that the announcement is here.
    pub fn take_early(&mut self, from: Fingerprint, id: u64) -> Option<Vec<u8>> {
        match &self.early {
            Some((peer, early, _)) if *peer == from && *early == id => self.early.take().map(|(_, _, data)| data),
            _ => None,
        }
    }

    /// Content arrived. Returns it if it is the newest and should be written.
    pub fn received(&mut self, from: Fingerprint, id: u64, data: &[u8]) -> Option<ClipAction> {
        match self.remote {
            Some((peer, newest, _)) if peer == from && newest == id => {}
            _ => {
                self.early = Some((from, id, data.to_vec()));
                return None;
            }
        }
        let content = ClipContent::decode(data)?;
        self.remote = None;
        self.busy = true;
        self.written_hash = Some(hash(data));
        self.last_gist = gist(&content);
        Some(ClipAction::Write { id, content })
    }

    /// We are about to write something of our own (finished with `write_done`).
    pub fn writing(&mut self) {
        self.busy = true;
    }

    /// Our write finished with this change count.
    pub fn write_done(&mut self, count: Option<u64>) {
        self.busy = false;
        if let Some(count) = count {
            self.seen = Some(count);
        }
    }
}

/// Sends an announcement (and the content, if small) to one peer.
pub fn announce(node: &Node, peer: &Fingerprint, offer: &Message, eager: Option<&Arc<Vec<u8>>>) {
    node.send(peer, offer.clone());
    if let (Message::ClipboardOffer { id, .. }, Some(data)) = (offer, eager) {
        node.send_blob(peer, header(*id), data.as_ref().clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: Fingerprint = Fingerprint([7; 32]);

    fn text(t: &str) -> Read {
        Read::Content(ClipContent {
            text: Some(t.into()),
            ..ClipContent::default()
        })
    }

    #[test]
    fn local_change_is_announced_once() {
        let mut sync = ClipSync::default();
        assert_eq!(sync.poll(5), None, "the content at start is not announced");
        assert_eq!(sync.poll(6), Some(ClipAction::Read));
        assert_eq!(sync.poll(7), None, "no second read while one runs");
        let (announce, skip) = sync.read_done(text("hi"), 1000);
        let (offer, eager) = announce.unwrap();
        assert!(matches!(offer, Message::ClipboardOffer { size: 2, .. }));
        assert!(eager.is_some());
        assert_eq!(skip, None);
        let Message::ClipboardOffer { id, .. } = offer else {
            unreachable!()
        };
        assert!(sync.requested(id).is_some());
        assert!(sync.requested(id + 1).is_none());
    }

    #[test]
    fn content_coming_back_from_another_sync_is_not_sent_again() {
        let mut sync = ClipSync::default();
        sync.poll(1);
        // Copied here and announced
        assert_eq!(sync.poll(2), Some(ClipAction::Read));
        assert!(sync.read_done(text("Grüße\n"), 1000).0.is_some());
        // Apple's Universal Clipboard brings it back with rich text and Windows line ends
        assert_eq!(sync.poll(3), Some(ClipAction::Read));
        let back = ClipContent {
            text: Some("Grüße\r\n".into()),
            rtf: Some(b"{\\rtf1 Gr\\u252e\\u223e}".to_vec()),
            ..ClipContent::default()
        };
        assert_eq!(sync.read_done(Read::Content(back), 1000), (None, None));
        // Something else is new, and the first text again after it as well
        assert_eq!(sync.poll(4), Some(ClipAction::Read));
        assert!(sync.read_done(text("other"), 1000).0.is_some());
        assert_eq!(sync.poll(5), Some(ClipAction::Read));
        assert!(sync.read_done(text("Grüße"), 1000).0.is_some());
    }

    #[test]
    fn written_content_is_not_announced_back() {
        let mut sync = ClipSync::default();
        sync.poll(1);
        assert_eq!(sync.offered(PEER, 42, 2, true), None);
        let content = ClipContent {
            text: Some("hi".into()),
            ..ClipContent::default()
        };
        let action = sync.received(PEER, 42, &content.encode()).unwrap();
        assert_eq!(
            action,
            ClipAction::Write {
                id: 42,
                content: content.clone()
            }
        );
        assert_eq!(sync.poll(2), None, "busy while writing");
        sync.write_done(Some(2));
        assert_eq!(sync.poll(2), None);
        // Some systems bump the counter twice; the hash still recognises our own content
        assert_eq!(sync.poll(3), Some(ClipAction::Read));
        assert_eq!(sync.read_done(Read::Content(content), 1000), (None, None));
    }

    #[test]
    fn large_content_is_fetched_when_needed() {
        let mut sync = ClipSync::default();
        let big = EAGER_LIMIT as u64 + 1;
        // Not in use here: nothing yet
        assert_eq!(sync.offered(PEER, 1, big, false), None);
        assert_eq!(sync.became_active(), Some((PEER, Message::ClipboardRequest { id: 1 })));
        assert_eq!(sync.became_active(), None, "asked only once");
        // In use: asked at once; old content that arrives late is ignored
        assert_eq!(
            sync.offered(PEER, 2, big, true),
            Some(Message::ClipboardRequest { id: 2 })
        );
        assert_eq!(sync.received(PEER, 1, &ClipContent::default().encode()), None);
        assert_eq!(
            sync.received(Fingerprint([8; 32]), 2, &ClipContent::default().encode()),
            None
        );
    }

    #[test]
    fn private_and_large_content_stays() {
        let mut sync = ClipSync::default();
        sync.poll(0);
        sync.poll(1);
        assert_eq!(sync.read_done(Read::Concealed, 1000), (None, Some(Skip::Concealed)));
        sync.poll(2);
        assert_eq!(sync.read_done(text("too long"), 3), (None, Some(Skip::TooLarge(8))));
        assert!(sync.for_new_peer().is_none());
    }

    #[test]
    fn content_before_its_announcement_is_kept() {
        let mut sync = ClipSync::default();
        let content = ClipContent {
            text: Some("early".into()),
            ..ClipContent::default()
        };
        assert_eq!(sync.received(PEER, 5, &content.encode()), None);
        sync.offered(PEER, 5, 5, true);
        let data = sync.take_early(PEER, 5).unwrap();
        assert!(matches!(
            sync.received(PEER, 5, &data),
            Some(ClipAction::Write { id: 5, .. })
        ));
        assert_eq!(sync.take_early(PEER, 5), None);
    }

    #[test]
    fn headers() {
        assert_eq!(parse_header(&header(77)), Some(77));
        assert_eq!(parse_header(b"clip"), None);
        assert_eq!(parse_header(b"file12345678"), None);
    }
}
