//! The native messaging host of the test extension (`extension/`), standing in for a desktop
//! app like 1Password's: each message, a 32-bit native-endian length and that much JSON on
//! stdin, is answered with the secret the extension fills in.

use std::io::{Read, Write};

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let mut len = [0u8; 4];
        if stdin.read_exact(&mut len).is_err() {
            return;
        }
        let mut message = vec![0u8; u32::from_ne_bytes(len) as usize];
        if stdin.read_exact(&mut message).is_err() {
            return;
        }
        let answer = br#"{"secret":"nd-secret-42"}"#;
        let _ = stdout.write_all(&(answer.len() as u32).to_ne_bytes());
        let _ = stdout.write_all(answer);
        let _ = stdout.flush();
    }
}
