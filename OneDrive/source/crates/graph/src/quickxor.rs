//! QuickXorHash, the content hash OneDrive reports for every file.
//!
//! Port of Microsoft's published reference implementation: a 160-bit
//! circular register where byte `k` is XORed in at bit `(k * 11) % 160`,
//! finished by XORing the little-endian length into the last 8 bytes.

use base64::Engine as _;
use std::io::Read;
use std::path::Path;

const WIDTH: usize = 160;
const SHIFT: usize = 11;
const BITS_IN_LAST_CELL: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct QuickXorHash {
    data: [u64; 3],
    length: u64,
    shift: usize,
}

impl QuickXorHash {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, input: &[u8]) {
        let mut index = self.shift / 64;
        let mut offset = self.shift % 64;
        let iterations = input.len().min(WIDTH);
        for i in 0..iterations {
            let is_last = index == self.data.len() - 1;
            let bits_in_cell = if is_last { BITS_IN_LAST_CELL } else { 64 };
            if offset <= bits_in_cell - 8 {
                let mut j = i;
                while j < input.len() {
                    self.data[index] ^= (input[j] as u64) << offset;
                    j += WIDTH;
                }
            } else {
                let next = if is_last { 0 } else { index + 1 };
                let low = bits_in_cell - offset;
                let mut xored = 0u8;
                let mut j = i;
                while j < input.len() {
                    xored ^= input[j];
                    j += WIDTH;
                }
                self.data[index] ^= (xored as u64) << offset;
                self.data[next] ^= (xored as u64) >> low;
            }
            offset += SHIFT;
            while offset >= bits_in_cell {
                index = if is_last { 0 } else { index + 1 };
                offset -= bits_in_cell;
            }
        }
        self.shift = (self.shift + SHIFT * (input.len() % WIDTH)) % WIDTH;
        self.length += input.len() as u64;
    }

    pub fn finalize(&self) -> [u8; 20] {
        let mut out = [0u8; 20];
        out[0..8].copy_from_slice(&self.data[0].to_le_bytes());
        out[8..16].copy_from_slice(&self.data[1].to_le_bytes());
        out[16..20].copy_from_slice(&self.data[2].to_le_bytes()[0..4]);
        for (i, b) in self.length.to_le_bytes().iter().enumerate() {
            out[12 + i] ^= b;
        }
        out
    }

    /// Base64 form, as found in `file.hashes.quickXorHash`.
    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.finalize())
    }

    pub fn of_file(path: &Path) -> std::io::Result<String> {
        let mut f = std::fs::File::open(path)?;
        let mut h = Self::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(h.base64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bit-by-bit model of the algorithm, used to check the optimised port.
    fn naive(data: &[u8]) -> [u8; 20] {
        let mut out = [0u8; 20];
        for (k, byte) in data.iter().enumerate() {
            let p = (k * SHIFT) % WIDTH;
            for b in 0..8 {
                if (byte >> b) & 1 == 1 {
                    let pos = (p + b) % WIDTH;
                    out[pos / 8] ^= 1 << (pos % 8);
                }
            }
        }
        for (i, b) in (data.len() as u64).to_le_bytes().iter().enumerate() {
            out[12 + i] ^= b;
        }
        out
    }

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(QuickXorHash::new().base64(), "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    }

    #[test]
    fn matches_bitwise_model_for_any_chunking() {
        for (len, seed) in [
            (1, 1),
            (7, 2),
            (159, 3),
            (160, 4),
            (161, 5),
            (1000, 6),
            (70_001, 7),
        ] {
            let data = pseudo_random(len, seed);
            let expected = naive(&data);
            let mut whole = QuickXorHash::new();
            whole.update(&data);
            assert_eq!(whole.finalize(), expected, "len {len}, single update");
            for chunk in [1, 3, 64, 159, 160, 333] {
                let mut h = QuickXorHash::new();
                for c in data.chunks(chunk) {
                    h.update(c);
                }
                assert_eq!(h.finalize(), expected, "len {len}, chunk {chunk}");
            }
        }
    }
}
