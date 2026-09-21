//! Trusting a server's certificate on first use, where no certificate
//! authority vouches for it.
//!
//! uberserver makes itself a self-signed X.509 *version 1* certificate on
//! first run (`certificate.py`), and every Spring lobby client before this
//! one trusted it by its fingerprint. webpki will not even read a version 1
//! certificate, so a server that fails the roots is checked here instead: its
//! certificate's SHA-256 is remembered the first time, and every later
//! connection must show the same one. A certificate the roots accept needs
//! none of this and is never pinned.

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::client::danger::{
	HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{
	WebPkiSupportedAlgorithms, verify_tls13_signature_with_raw_key,
};
use tokio_rustls::rustls::pki_types::{
	CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime,
};
use tokio_rustls::rustls::{
	CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};

/// The SHA-256 of a server's certificate, as the server sent it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
	pub fn of(certificate: &[u8]) -> Self {
		Self(Sha256::digest(certificate).into())
	}

	/// Enough of it to tell two apart in a message.
	pub fn short(&self) -> String {
		self.to_string()[..16].to_owned()
	}
}

impl fmt::Display for Fingerprint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
	}
}

impl fmt::Debug for Fingerprint {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Fingerprint({self})")
	}
}

impl Serialize for Fingerprint {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_str(self)
	}
}

impl<'de> Deserialize<'de> for Fingerprint {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let text = String::deserialize(deserializer)?;
		let mut bytes = [0u8; 32];
		let valid = text.len() == 64
			&& bytes.iter_mut().enumerate().all(|(at, byte)| {
				u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).is_ok_and(|read| {
					*byte = read;
					true
				})
			});
		valid
			.then_some(Self(bytes))
			.ok_or_else(|| serde::de::Error::custom("a fingerprint is 64 hex digits"))
	}
}

/// What a connection will take for a certificate the roots refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trust {
	/// Nothing: the roots' word is final.
	Roots,
	/// Whatever it is, this once, and that one from then on.
	FirstUse,
	/// This one only. `None` is a server the roots vouched for last time, so
	/// any certificate they refuse now is a change.
	Pinned(Option<Fingerprint>),
}

/// What became of the certificate a connection was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen {
	/// The roots vouched for it.
	Verified,
	/// Trusted by its fingerprint, first time or as remembered.
	Pinned(Fingerprint),
	/// Not the one trusted before.
	Changed {
		was: Option<Fingerprint>,
		now: Fingerprint,
	},
}

type Outcome = Arc<Mutex<Option<Seen>>>;

/// A connector for one attempt, and where it says what it made of the
/// certificate. A fresh verifier each time: what it saw is this attempt's.
pub(crate) fn connector(trust: Trust) -> (TlsConnector, Outcome) {
	let outcome = Outcome::default();
	let verifier = Pinning {
		roots: roots(),
		trust,
		outcome: Arc::clone(&outcome),
	};
	let config = ClientConfig::builder()
		.dangerous()
		.with_custom_certificate_verifier(Arc::new(verifier))
		.with_no_client_auth();
	(TlsConnector::from(Arc::new(config)), outcome)
}

/// The Mozilla root store bundled by `webpki-roots`, built once: a race makes
/// a handshake per port per way, and the store is the same for all of them.
fn roots() -> Arc<WebPkiServerVerifier> {
	static ROOTS: OnceLock<Arc<WebPkiServerVerifier>> = OnceLock::new();
	Arc::clone(ROOTS.get_or_init(|| {
		crate::transport::install_crypto();
		let mut store = RootCertStore::empty();
		store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
		WebPkiServerVerifier::builder(Arc::new(store))
			.build()
			.expect("the bundled roots make a verifier")
	}))
}

fn algorithms() -> WebPkiSupportedAlgorithms {
	tokio_rustls::rustls::crypto::ring::default_provider().signature_verification_algorithms
}

#[derive(Debug)]
struct Pinning {
	roots: Arc<WebPkiServerVerifier>,
	trust: Trust,
	outcome: Outcome,
}

impl Pinning {
	fn saw(&self, seen: Seen) {
		*self.outcome.lock().expect("pin outcome lock") = Some(seen);
	}

	fn verified(&self) -> bool {
		*self.outcome.lock().expect("pin outcome lock") == Some(Seen::Verified)
	}
}

impl ServerCertVerifier for Pinning {
	fn verify_server_cert(
		&self,
		end_entity: &CertificateDer<'_>,
		intermediates: &[CertificateDer<'_>],
		server_name: &ServerName<'_>,
		ocsp_response: &[u8],
		now: UnixTime,
	) -> Result<ServerCertVerified, Error> {
		let refused = match self.roots.verify_server_cert(
			end_entity,
			intermediates,
			server_name,
			ocsp_response,
			now,
		) {
			Ok(verified) => {
				self.saw(Seen::Verified);
				return Ok(verified);
			}
			Err(refused) => refused,
		};
		let shown = Fingerprint::of(end_entity);
		match self.trust {
			Trust::Roots => Err(refused),
			Trust::Pinned(Some(pin)) if pin == shown => {
				self.saw(Seen::Pinned(shown));
				Ok(ServerCertVerified::assertion())
			}
			Trust::FirstUse => {
				self.saw(Seen::Pinned(shown));
				Ok(ServerCertVerified::assertion())
			}
			Trust::Pinned(was) => {
				self.saw(Seen::Changed { was, now: shown });
				Err(refused)
			}
		}
	}

	// ponytail: a pinned certificate is only checked against a TLS 1.3
	// handshake, which is all rustls can do with a bare key; every uberserver
	// seen (Recoil, Metal Factions, 2026-09-21) speaks 1.3.
	fn verify_tls12_signature(
		&self,
		message: &[u8],
		cert: &CertificateDer<'_>,
		dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, Error> {
		if self.verified() {
			return self.roots.verify_tls12_signature(message, cert, dss);
		}
		Err(Error::General(
			"a certificate trusted on first use needs TLS 1.3".into(),
		))
	}

	fn verify_tls13_signature(
		&self,
		message: &[u8],
		cert: &CertificateDer<'_>,
		dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, Error> {
		if self.verified() {
			return self.roots.verify_tls13_signature(message, cert, dss);
		}
		let key = spki(cert).ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))?;
		verify_tls13_signature_with_raw_key(
			message,
			&SubjectPublicKeyInfoDer::from(key),
			dss,
			&algorithms(),
		)
	}

	fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
		self.roots.supported_verify_schemes()
	}
}

/// A certificate's SubjectPublicKeyInfo, whole, read without judging the rest
/// of it -- which is the point: webpki refuses a version 1 certificate before
/// it gets as far as the key.
fn spki(certificate: &[u8]) -> Option<&[u8]> {
	let (_, certificate, _) = tlv(certificate)?;
	let (_, tbs, _) = tlv(certificate)?;
	let mut rest = tbs;
	// `[0] version`, present from version 2 on.
	if let Some((0xA0, _, after)) = tlv(rest) {
		rest = after;
	}
	// serialNumber, signature, issuer, validity, subject.
	for _ in 0..5 {
		rest = tlv(rest)?.2;
	}
	let start = rest;
	let (tag, _, after) = tlv(rest)?;
	(tag == 0x30).then(|| &start[..start.len() - after.len()])
}

/// One DER element: its tag, its contents, and what follows it.
fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
	let (&tag, rest) = input.split_first()?;
	let (&first, rest) = rest.split_first()?;
	let (length, rest) = if first < 0x80 {
		(usize::from(first), rest)
	} else {
		let count = usize::from(first & 0x7f);
		if count == 0 || count > 4 || rest.len() < count {
			return None;
		}
		let (digits, rest) = rest.split_at(count);
		let length = digits
			.iter()
			.fold(0usize, |length, digit| length << 8 | usize::from(*digit));
		(length, rest)
	};
	(rest.len() >= length).then(|| (tag, &rest[..length], &rest[length..]))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// lobby.recoilengine.org's, 2026-09-21: uberserver's own, version 1.
	const UBERSERVER: &[u8] = include_bytes!("../testdata/uberserver-v1.der");
	/// Its key, as `openssl x509 -pubkey | openssl pkey -pubin -outform der` reads it.
	const UBERSERVER_KEY: &[u8] = include_bytes!("../testdata/uberserver-v1.spki.der");

	fn check(trust: Trust) -> (Result<ServerCertVerified, Error>, Option<Seen>) {
		let outcome = Outcome::default();
		let verifier = Pinning {
			roots: roots(),
			trust,
			outcome: Arc::clone(&outcome),
		};
		let result = verifier.verify_server_cert(
			&CertificateDer::from(UBERSERVER),
			&[],
			&ServerName::try_from("lobby.recoilengine.org").unwrap(),
			&[],
			UnixTime::now(),
		);
		let seen = *outcome.lock().unwrap();
		(result, seen)
	}

	#[test]
	fn the_key_is_read_out_of_a_version_1_certificate() {
		assert_eq!(spki(UBERSERVER), Some(UBERSERVER_KEY));
		assert_eq!(spki(&UBERSERVER[..100]), None);
		assert_eq!(spki(b""), None);
	}

	#[test]
	fn a_certificate_the_roots_refuse_is_trusted_the_first_time_and_then_only_as_it_was() {
		let shown = Fingerprint::of(UBERSERVER);
		assert_eq!(
			shown.to_string(),
			"577267e2c7834b0aaebf6db78f7e460e875aa0c8e3c5e2820c671d5afdbaecd1"
		);

		let (first, seen) = check(Trust::FirstUse);
		assert!(first.is_ok());
		assert_eq!(seen, Some(Seen::Pinned(shown)));

		let (again, seen) = check(Trust::Pinned(Some(shown)));
		assert!(again.is_ok());
		assert_eq!(seen, Some(Seen::Pinned(shown)));

		let other = Fingerprint([7; 32]);
		let (changed, seen) = check(Trust::Pinned(Some(other)));
		assert!(changed.is_err());
		assert_eq!(
			seen,
			Some(Seen::Changed {
				was: Some(other),
				now: shown
			})
		);
	}

	#[test]
	fn a_server_the_roots_vouched_for_is_not_trusted_on_first_use_after() {
		let (vouched_before, seen) = check(Trust::Pinned(None));
		assert!(vouched_before.is_err());
		assert!(matches!(seen, Some(Seen::Changed { was: None, .. })));

		let (roots_only, seen) = check(Trust::Roots);
		assert!(roots_only.is_err());
		assert_eq!(seen, None);
	}

	#[test]
	fn a_fingerprint_is_kept_as_hex_and_nothing_else_reads_as_one() {
		let pin = Fingerprint::of(UBERSERVER);
		let text = serde_json::to_string(&pin).unwrap();
		assert_eq!(serde_json::from_str::<Fingerprint>(&text).unwrap(), pin);
		assert!(serde_json::from_str::<Fingerprint>("\"abc\"").is_err());
		assert!(serde_json::from_str::<Fingerprint>(&format!("\"{}\"", "zz".repeat(32))).is_err());
		assert_eq!(pin.short(), "577267e2c7834b0a");
	}
}
