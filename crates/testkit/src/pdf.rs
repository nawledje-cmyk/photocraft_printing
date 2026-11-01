//! Synthetic PDFs for tests: `pages` pages of `w`×`h` points, each with a red square, optionally
//! password-protected with the PDF Standard Security Handler, revision 2 (40-bit RC4; ISO
//! 32000-1 §7.6.3, algorithms 2, 3 and 4), written from the spec for these tests.

/// The 32-byte password padding of ISO 32000-1 §7.6.3.3.
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C,
    0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// A PDF of `pages` pages, `w`×`h` points, each with a red square at (10, 10)–(50, 50); with
/// `password`, encrypted so only that password opens it.
pub fn pdf(w: u32, h: u32, pages: usize, password: Option<&str>) -> Vec<u8> {
    let id = *b"photocraft-test!";
    let crypt = password.map(|pw| Crypt::new(pw.as_bytes(), &id));
    let content = b"1 0 0 rg 10 10 40 40 re f".to_vec();
    let mut objs: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")).into_bytes());
    for i in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + i * 2).into_bytes());
        let num = (4 + i * 2) as u32;
        let data = match &crypt {
            Some(c) => c.encrypt(num, &content),
            None => content.clone(),
        };
        let mut o = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
        o.extend(&data);
        o.extend(b"\nendstream");
        objs.push(o);
    }
    if let Some(c) = &crypt {
        objs.push(format!("<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P -4 >>", hex(&c.o), hex(&c.u)).into_bytes());
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    let encrypt = if crypt.is_some() { format!(" /Encrypt {} 0 R", objs.len()) } else { String::new() };
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R{encrypt} /ID [<{}> <{}>] >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1, hex(&id), hex(&id)).bytes());
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn padded(pw: &[u8]) -> [u8; 32] {
    let mut p = PAD;
    let n = pw.len().min(32);
    p[..n].copy_from_slice(&pw[..n]);
    p[n..].copy_from_slice(&PAD[..32 - n]);
    p
}

/// Revision 2 keys and the /O and /U entries for a user password (owner password the same).
struct Crypt {
    key: [u8; 5],
    o: [u8; 32],
    u: [u8; 32],
}

impl Crypt {
    fn new(pw: &[u8], id: &[u8]) -> Self {
        // Algorithm 3: /O from the (owner = user) password.
        let owner_key = md5(&padded(pw));
        let o_full = rc4(&owner_key[..5], &padded(pw));
        let mut o = [0u8; 32];
        o.copy_from_slice(&o_full);
        // Algorithm 2: the file key.
        let mut input = padded(pw).to_vec();
        input.extend(o);
        input.extend((-4i32).to_le_bytes());
        input.extend(id);
        let digest = md5(&input);
        let mut key = [0u8; 5];
        key.copy_from_slice(&digest[..5]);
        // Algorithm 4: /U.
        let mut u = [0u8; 32];
        u.copy_from_slice(&rc4(&key, &PAD));
        Self { key, o, u }
    }

    /// Algorithm 1: an object's data, encrypted with its own key.
    fn encrypt(&self, num: u32, data: &[u8]) -> Vec<u8> {
        let mut k = self.key.to_vec();
        k.extend(&num.to_le_bytes()[..3]);
        k.extend([0, 0]);
        let d = md5(&k);
        rc4(&d[..10], data)
    }
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

/// MD5 (RFC 1321).
fn md5(msg: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut m = msg.to_vec();
    let bits = (msg.len() as u64).wrapping_mul(8);
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend(bits.to_le_bytes());
    for chunk in m.chunks(64) {
        let w: Vec<u32> = chunk.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(w[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn md5_and_rc4_match_their_published_vectors() {
        let hex = |b: &[u8]| super::hex(b).to_lowercase();
        assert_eq!(hex(&super::md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&super::md5(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(hex(&super::rc4(b"Key", b"Plaintext")), "bbf316e8d940af0ad3");
    }
}
