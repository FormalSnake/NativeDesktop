//! `.crx` packages. Chromium installs a package only from the Web Store, so one the app hands
//! over is unpacked here and loaded as an unpacked extension, with the package's public key
//! written into its manifest: Chromium derives an unpacked extension's id from that key, so it
//! keeps the id the store gave it (and with it its storage and native messaging allowlists).
//!
//! CRX3: `Cr24`, version 3, the header's length, a `CrxFileHeader` protobuf, then a zip.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// Unpacks `crx` into `<into>/<id>` (replacing an older copy) and returns that directory.
pub(crate) fn unpack(crx: &Path, into: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(crx).map_err(|e| format!("{}: {e}", crx.display()))?;
    let (key, zip) = parse(&bytes).map_err(|e| format!("{}: {e}", crx.display()))?;
    let id = id_for_key(&key);
    let dir = into.join(&id);
    let staging = into.join(format!(".{id}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    archive.extract(&staging).map_err(|e| e.to_string())?;
    let manifest_path = staging.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| format!("manifest.json: {e}"))?;
    let mut manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("manifest.json: {e}"))?;
    manifest["key"] = serde_json::Value::String(STANDARD.encode(&key));
    std::fs::write(&manifest_path, manifest.to_string()).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The id Chromium gives an extension with public key `key` (DER): the first 16 bytes of its
/// SHA-256, each nibble a letter from `a`.
pub(crate) fn id_for_key(key: &[u8]) -> String {
    Sha256::digest(key)[..16].iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect()
}

/// The public key that signed the package (the one whose id the header names) and the zip.
fn parse(bytes: &[u8]) -> Result<(Vec<u8>, &[u8]), String> {
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(..4) != Some(b"Cr24") {
        return Err("not a CRX package".into());
    }
    if u32_at(4) != Some(3) {
        return Err("only CRX3 packages are supported".into());
    }
    let len = u32_at(8).ok_or("truncated header")? as usize;
    let header = bytes.get(12..12 + len).ok_or("truncated header")?;
    let zip = &bytes[12 + len..];
    let mut keys = Vec::new();
    let mut crx_id = None;
    for (field, value) in fields(header)? {
        match field {
            // sha256_with_rsa and sha256_with_ecdsa: AsymmetricKeyProof { public_key = 1 }.
            2 | 3 => {
                for (f, v) in fields(value)? {
                    if f == 1 {
                        keys.push(v.to_vec());
                    }
                }
            }
            // signed_header_data: SignedData { crx_id = 1 }.
            10000 => {
                for (f, v) in fields(value)? {
                    if f == 1 {
                        crx_id = Some(v.to_vec());
                    }
                }
            }
            _ => {}
        }
    }
    let crx_id = crx_id.ok_or("the header names no id")?;
    let key = keys
        .into_iter()
        .find(|k| Sha256::digest(k)[..16] == crx_id[..])
        .ok_or("no key in the header matches its id")?;
    Ok((key, zip))
}

/// The length-delimited fields of a protobuf message as (number, bytes); other wire types are
/// skipped.
fn fields(mut b: &[u8]) -> Result<Vec<(u64, &[u8])>, String> {
    fn varint(b: &mut &[u8]) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = b.split_first().ok_or("truncated varint")?;
            *b = rest;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("varint too long".into())
    }
    let mut out = Vec::new();
    while !b.is_empty() {
        let tag = varint(&mut b)?;
        match tag & 7 {
            0 => {
                varint(&mut b)?;
            }
            1 => b = b.get(8..).ok_or("truncated field")?,
            2 => {
                let len = varint(&mut b)? as usize;
                let value = b.get(..len).ok_or("truncated field")?;
                out.push((tag >> 3, value));
                b = &b[len..];
            }
            5 => b = b.get(4..).ok_or("truncated field")?,
            _ => return Err("unsupported protobuf wire type".into()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(number: u64, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut tag = (number << 3) | 2;
        loop {
            let byte = (tag & 0x7f) as u8;
            tag >>= 7;
            out.push(if tag == 0 { byte } else { byte | 0x80 });
            if tag == 0 {
                break;
            }
        }
        out.push(value.len() as u8);
        out.extend_from_slice(value);
        out
    }

    #[test]
    fn a_package_keeps_the_id_its_key_gives() {
        let key = b"not really a DER key, but hashed the same way".to_vec();
        let id = Sha256::digest(&key)[..16].to_vec();
        let mut header = field(2, &[field(1, &key), field(2, b"sig")].concat());
        header.extend(field(10000, &field(1, &id)));
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend((header.len() as u32).to_le_bytes());
        crx.extend(&header);
        crx.extend(b"PK zip");
        let (found, zip) = parse(&crx).unwrap();
        assert_eq!(found, key);
        assert_eq!(zip, b"PK zip");
        assert_eq!(id_for_key(&key).len(), 32);
        assert!(id_for_key(&key).chars().all(|c| ('a'..='p').contains(&c)));
    }

    #[test]
    fn other_files_are_refused() {
        assert!(parse(b"PK\x03\x04").is_err());
        assert!(parse(b"Cr24\x02\0\0\0").is_err());
    }
}
