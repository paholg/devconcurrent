use crate::client::Docker;
use crate::error::Result;
use crate::request_ext::ReqwestExt;

impl Docker {
    /// `PUT /containers/{id}/archive?path=<dest>` — extract a tar archive into
    /// `dest` inside the container.
    pub async fn upload_archive(&self, id: &str, dest_dir: &str, tar: Vec<u8>) -> Result<()> {
        let mut url = self.url(&format!("containers/{id}/archive"));
        url.query_pairs_mut().append_pair("path", dest_dir);
        self.http()
            .put(url)
            .header("Content-Type", "application/x-tar")
            .body(tar)
            .try_send_empty()
            .await
    }
}

/// Build a tar archive containing exactly one regular file.
///
/// `filename` is stored as the entry name (no path components). `mtime` is set
/// to 0; `mode` is `0o644`. The output is a complete archive including the two
/// trailing zero blocks tar(1) expects as an end-of-archive marker.
pub fn build_single_file_tar(filename: &str, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(512 + round_up_512(content.len()) + 1024);
    out.extend_from_slice(&ustar_header(filename, content.len()));
    out.extend_from_slice(content);
    let pad = round_up_512(content.len()) - content.len();
    out.extend(std::iter::repeat_n(0, pad));
    out.extend(std::iter::repeat_n(0, 1024));
    out
}

fn round_up_512(n: usize) -> usize {
    (n + 511) & !511
}

fn ustar_header(filename: &str, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];

    let name = filename.as_bytes();
    assert!(name.len() <= 100, "tar entry name too long: {filename:?}");
    h[..name.len()].copy_from_slice(name);

    write_octal(&mut h[100..108], 0o644, 8);
    write_octal(&mut h[108..116], 0, 8);
    write_octal(&mut h[116..124], 0, 8);
    write_octal(&mut h[124..136], size as u64, 12);
    write_octal(&mut h[136..148], 0, 12);

    // chksum: 8 spaces while computing.
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0'; // typeflag: regular file
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");

    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    let chk = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(chk.as_bytes());

    h
}

fn write_octal(buf: &mut [u8], mut value: u64, width: usize) {
    // Numeric fields are width-1 octal digits, zero-padded, followed by a NUL.
    let mut digits = vec![b'0'; width - 1];
    let mut i = digits.len();
    while value > 0 && i > 0 {
        i -= 1;
        digits[i] = b'0' + ((value & 0o7) as u8);
        value >>= 3;
    }
    buf[..width - 1].copy_from_slice(&digits);
    buf[width - 1] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout() {
        let tar = build_single_file_tar("hello.txt", b"hi\n");
        // Header starts with the name.
        assert_eq!(&tar[..9], b"hello.txt");
        // Magic at offset 257.
        assert_eq!(&tar[257..263], b"ustar\0");
        // Content directly after the 512-byte header.
        assert_eq!(&tar[512..515], b"hi\n");
        // Total size: 512 (header) + 512 (content padded) + 1024 (eof) = 2048.
        assert_eq!(tar.len(), 2048);
    }

    #[test]
    fn checksum_is_octal_six_digits() {
        let tar = build_single_file_tar("a", b"x");
        let chk = std::str::from_utf8(&tar[148..154]).unwrap();
        assert!(chk.chars().all(|c| c.is_ascii_digit() && c < '8'));
    }

    #[test]
    fn larger_content_padded_to_512() {
        let content = vec![b'a'; 600];
        let tar = build_single_file_tar("a", &content);
        // 512 header + 1024 (600 → next 512 boundary) + 1024 eof = 2560
        assert_eq!(tar.len(), 2560);
        // First 600 bytes after header are content; next 424 are zeros.
        assert!(tar[512..1112].iter().all(|&b| b == b'a'));
        assert!(tar[1112..1536].iter().all(|&b| b == 0));
    }
}
