//! Pairing with a 6-digit code.
//!
//! One computer shows a code, the other one types it in. Both run SPAKE2 with the code as password and
//! the two certificate fingerprints from the TLS handshake as identities. Only if both know the code
//! *and* see the same two certificates do they end up with the same key, which each side proves with
//! an HMAC before it stores the other one. A machine in the middle would present its own certificate
//! to both sides and fail that check; guessing the code costs one attempt per connection, and the
//! showing side stops after `MAX_ATTEMPTS` wrong ones.

use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use tc_protocol::PROTOCOL_VERSION;

use crate::identity::Fingerprint;

/// How long a shown code stays valid.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Wrong attempts until the code is void.
pub const MAX_ATTEMPTS: u8 = 5;

const CONFIRM_SHOWING: &[u8] = b"tablecross pairing: code shown here";
const CONFIRM_TYPING: &[u8] = b"tablecross pairing: code typed here";

/// A code as shown to the user: six digits.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode(String);

impl PairingCode {
    pub fn random() -> Self {
        // Rejection sampling keeps all codes equally likely
        loop {
            let mut bytes = [0u8; 4];
            getrandom::fill(&mut bytes).expect("system random generator");
            let n = u32::from_le_bytes(bytes);
            if n < 4_294_000_000 {
                return Self(format!("{:06}", n % 1_000_000));
            }
        }
    }

    /// Accepts what a person types: digits with optional spaces or dashes.
    pub fn parse(input: &str) -> Option<Self> {
        let digits: String = input.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
        (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then_some(Self(digits))
    }

    pub fn digits(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PairingCode {
    /// "123 456", easier to read aloud
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", &self.0[..3], &self.0[3..])
    }
}

impl std::fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairingCode(***)")
    }
}

/// The code currently shown on this computer.
#[derive(Debug)]
pub(crate) struct PairingWindow {
    pub code: PairingCode,
    pub until: Instant,
    pub attempts_left: u8,
}

impl PairingWindow {
    pub fn new(code: PairingCode) -> Self {
        Self {
            code,
            until: Instant::now() + PAIRING_WINDOW,
            attempts_left: MAX_ATTEMPTS,
        }
    }

    /// Uses up one attempt; `None` if the window is closed.
    pub fn take_attempt(&mut self) -> Option<PairingCode> {
        if self.attempts_left == 0 || Instant::now() >= self.until {
            return None;
        }
        self.attempts_left -= 1;
        Some(self.code.clone())
    }
}

/// Messages on the pairing stream.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum PairMsg {
    /// From the typing side.
    Start { version: u16, name: String, spake: Vec<u8> },
    /// From the showing side: its SPAKE2 message and proof of the key.
    Reply {
        name: String,
        spake: Vec<u8>,
        confirm: [u8; 32],
    },
    /// From the typing side: its proof of the key.
    Confirm { confirm: [u8; 32] },
}

impl PairMsg {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("pairing messages always encode")
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        match postcard::take_from_bytes(bytes) {
            Ok((msg, [])) => Some(msg),
            _ => None,
        }
    }
}

/// SPAKE2 state of one pairing attempt. The typing side is "A", the showing side "B".
pub(crate) struct Exchange {
    spake: Spake2<Ed25519Group>,
    pub outgoing: Vec<u8>,
    typing: Fingerprint,
    showing: Fingerprint,
}

impl Exchange {
    pub fn typing(code: &PairingCode, typing: Fingerprint, showing: Fingerprint) -> Self {
        let (spake, outgoing) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.digits().as_bytes()),
            &SpakeIdentity::new(&typing.0),
            &SpakeIdentity::new(&showing.0),
        );
        Self {
            spake,
            outgoing,
            typing,
            showing,
        }
    }

    pub fn showing(code: &PairingCode, typing: Fingerprint, showing: Fingerprint) -> Self {
        let (spake, outgoing) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.digits().as_bytes()),
            &SpakeIdentity::new(&typing.0),
            &SpakeIdentity::new(&showing.0),
        );
        Self {
            spake,
            outgoing,
            typing,
            showing,
        }
    }

    /// Combines with the other side's message. A malformed message gives `None`.
    pub fn finish(self, incoming: &[u8]) -> Option<Keys> {
        let key = self.spake.finish(incoming).ok()?;
        Some(Keys {
            key,
            typing: self.typing,
            showing: self.showing,
        })
    }
}

pub(crate) struct Keys {
    key: Vec<u8>,
    typing: Fingerprint,
    showing: Fingerprint,
}

impl Keys {
    fn mac(&self, label: &[u8]) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes any key length");
        mac.update(label);
        mac.update(&self.typing.0);
        mac.update(&self.showing.0);
        mac.update(&PROTOCOL_VERSION.to_le_bytes());
        mac
    }

    pub fn showing_proof(&self) -> [u8; 32] {
        self.mac(CONFIRM_SHOWING).finalize().into_bytes().into()
    }

    pub fn typing_proof(&self) -> [u8; 32] {
        self.mac(CONFIRM_TYPING).finalize().into_bytes().into()
    }

    /// Constant-time comparison.
    pub fn check_showing(&self, proof: &[u8; 32]) -> bool {
        self.mac(CONFIRM_SHOWING).verify_slice(proof).is_ok()
    }

    pub fn check_typing(&self, proof: &[u8; 32]) -> bool {
        self.mac(CONFIRM_TYPING).verify_slice(proof).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Fingerprint = Fingerprint([1; 32]);
    const S: Fingerprint = Fingerprint([2; 32]);

    fn run(typed: &str, shown: &str, typing_sees: (Fingerprint, Fingerprint)) -> (bool, bool) {
        let a = Exchange::typing(&PairingCode::parse(typed).unwrap(), typing_sees.0, typing_sees.1);
        let b = Exchange::showing(&PairingCode::parse(shown).unwrap(), T, S);
        let (a_out, b_out) = (a.outgoing.clone(), b.outgoing.clone());
        let ka = a.finish(&b_out).unwrap();
        let kb = b.finish(&a_out).unwrap();
        (
            ka.check_showing(&kb.showing_proof()),
            kb.check_typing(&ka.typing_proof()),
        )
    }

    #[test]
    fn same_code_same_certificates_agree() {
        assert_eq!(run("123456", "123 456", (T, S)), (true, true));
    }

    #[test]
    fn wrong_code_fails() {
        assert_eq!(run("123457", "123456", (T, S)), (false, false));
    }

    #[test]
    fn different_certificates_fail() {
        // What a machine in the middle would cause: the typing side sees another certificate
        assert_eq!(run("123456", "123456", (T, Fingerprint([9; 32]))), (false, false));
    }

    #[test]
    fn proofs_are_not_interchangeable() {
        let a = Exchange::typing(&PairingCode::parse("000000").unwrap(), T, S);
        let b = Exchange::showing(&PairingCode::parse("000000").unwrap(), T, S);
        let (a_out, b_out) = (a.outgoing.clone(), b.outgoing.clone());
        let ka = a.finish(&b_out).unwrap();
        let kb = b.finish(&a_out).unwrap();
        // Reflecting the other side's proof back must not work
        assert!(!kb.check_typing(&kb.showing_proof()));
        assert!(!ka.check_showing(&ka.typing_proof()));
    }

    #[test]
    fn codes() {
        for _ in 0..200 {
            let code = PairingCode::random();
            assert_eq!(code.digits().len(), 6);
            assert_eq!(PairingCode::parse(&code.to_string()), Some(code));
        }
        assert_eq!(PairingCode::parse("12-34-56").unwrap().digits(), "123456");
        for bad in ["12345", "1234567", "12345a", "", "１２３４５６"] {
            assert!(PairingCode::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn window_counts_attempts() {
        let mut window = PairingWindow::new(PairingCode::random());
        for _ in 0..MAX_ATTEMPTS {
            assert!(window.take_attempt().is_some());
        }
        assert!(window.take_attempt().is_none());
        let mut expired = PairingWindow::new(PairingCode::random());
        expired.until = Instant::now() - Duration::from_secs(1);
        assert!(expired.take_attempt().is_none());
    }

    #[test]
    fn garbage_pairing_messages_are_rejected() {
        assert!(PairMsg::decode(&[]).is_none());
        assert!(PairMsg::decode(&[0xFF; 40]).is_none());
        let mut ok = PairMsg::Confirm { confirm: [0; 32] }.encode();
        assert!(PairMsg::decode(&ok).is_some());
        ok.push(0);
        assert!(PairMsg::decode(&ok).is_none());
        let bad = Exchange::typing(&PairingCode::random(), T, S);
        assert!(bad.finish(&[1, 2, 3]).is_none());
    }
}
