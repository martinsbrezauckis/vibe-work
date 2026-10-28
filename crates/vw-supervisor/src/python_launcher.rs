//! Reads interpreter associations at the defined locations in Windows uv and distlib launchers.
//! Bounds-checked PE resource and ZIP-trailer parsing only; called by the Vibe interpreter resolver.
use std::path::PathBuf;

fn u16_at(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(usize::from(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    )))
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
    .ok()
}

struct Pe<'a> {
    bytes: &'a [u8],
    sections: usize,
    count: usize,
    resource: usize,
    end: usize,
}
impl<'a> Pe<'a> {
    fn read(bytes: &'a [u8]) -> Option<Self> {
        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(bytes, 0x3c)?;
        if bytes.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let count = u16_at(bytes, pe + 6)?;
        if !(1..=96).contains(&count) {
            return None;
        }
        let optional = pe + 24;
        let size = u16_at(bytes, pe + 20)?;
        let directory = match u16_at(bytes, optional)? {
            0x10b => 96,
            0x20b => 112,
            _ => return None,
        };
        if size < directory + 24 {
            return None;
        }
        let sections = optional + size;
        let mut result = Self {
            bytes,
            sections,
            count,
            resource: u32_at(bytes, optional + directory + 16)?,
            end: sections + count * 40,
        };
        for i in 0..count {
            let row = sections + i * 40;
            let end = u32_at(bytes, row + 20)?.checked_add(u32_at(bytes, row + 16)?)?;
            bytes.get(..end)?;
            result.end = result.end.max(end);
        }
        Some(result)
    }
    fn rva(&self, address: usize, length: usize) -> Option<usize> {
        for i in 0..self.count {
            let row = self.sections + i * 40;
            let start = u32_at(self.bytes, row + 12)?;
            let size = u32_at(self.bytes, row + 16)?;
            if let Some(offset) = address.checked_sub(start)
                && offset.checked_add(length)? <= size
            {
                return u32_at(self.bytes, row + 20)?.checked_add(offset);
            }
        }
        None
    }
    fn resource(&self, name: &str) -> Option<&'a [u8]> {
        let base = self.rva(self.resource, 16)?;
        let mut table = 0;
        for level in 0..3 {
            let at = base.checked_add(table)?;
            let count = u16_at(self.bytes, at + 12)?.checked_add(u16_at(self.bytes, at + 14)?)?;
            if count > 256 {
                return None;
            }
            let mut found = None;
            for i in 0..count {
                let entry = at + 16 + i * 8;
                let key = u32_at(self.bytes, entry)?;
                let matches = match level {
                    0 => key == 10, // RT_RCDATA
                    1 if key & 0x8000_0000 != 0 => {
                        let at = base.checked_add(key & 0x7fff_ffff)?;
                        let length = u16_at(self.bytes, at)?;
                        let text = self.bytes.get(at + 2..(at + 2).checked_add(length * 2)?)?;
                        text.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                            .eq(name.encode_utf16())
                    }
                    2 => key == 0, // uv writes the neutral language.
                    _ => false,
                };
                if matches {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(u32_at(self.bytes, entry + 4)?);
                }
            }
            let next = found?;
            if (next & 0x8000_0000 != 0) != (level < 2) {
                return None;
            }
            table = next & 0x7fff_ffff;
        }
        let data = base.checked_add(table)?;
        let length = u32_at(self.bytes, data + 4)?;
        let offset = self.rva(u32_at(self.bytes, data)?, length)?;
        self.bytes.get(offset..offset.checked_add(length)?)
    }
}

fn python_path(bytes: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(bytes).ok()?;
    let b = text.as_bytes();
    if b.len() < 3
        || !b[0].is_ascii_alphabetic()
        || b[1] != b':'
        || !matches!(b[2], b'\\' | b'/')
        || b.iter()
            .any(|b| *b < 0x20 || matches!(*b, b'"' | b'<' | b'>' | b'|' | b'*' | b'?'))
        || !text
            .rsplit(['\\', '/'])
            .next()?
            .eq_ignore_ascii_case("python.exe")
    {
        return None;
    }
    Some(PathBuf::from(text))
}

pub(super) fn interpreter(bytes: &[u8]) -> Option<PathBuf> {
    let pe = Pe::read(bytes)?;
    if let Some(kind) = pe.resource("UV_TRAMPOLINE_KIND") {
        if kind != [1] || !pe.resource("UV_SCRIPT_DATA")?.starts_with(b"PK\x03\x04") {
            return None;
        }
        return python_path(pe.resource("UV_PYTHON_PATH")?);
    }
    // distlib emits PE + shebang + ZIP. The shebang must start exactly at the PE overlay, and the
    // ZIP directory must resolve to the byte immediately after it, never an arbitrary embedded string.
    let trailer = bytes.get(pe.end..)?.strip_prefix(b"#!")?;
    let newline = trailer.iter().position(|b| *b == b'\n')?;
    let line = trailer
        .get(..newline)?
        .strip_suffix(b"\r")
        .unwrap_or(&trailer[..newline]);
    let line = if line.starts_with(b"\"") {
        line.strip_prefix(b"\"")?.strip_suffix(b"\"")?
    } else {
        line
    };
    let zip = pe.end + 2 + newline + 1;
    if bytes.get(zip..zip + 4)? != b"PK\x03\x04" {
        return None;
    }
    let end = bytes.len().checked_sub(22)?;
    if bytes.get(end..end + 4)? != b"PK\x05\x06" || u16_at(bytes, end + 20)? != 0 {
        return None;
    }
    let central = end.checked_sub(u32_at(bytes, end + 12)?)?;
    if central.checked_sub(u32_at(bytes, end + 16)?)? != zip
        || bytes.get(central..central + 4)? != b"PK\x01\x02"
    {
        return None;
    }
    python_path(line)
}
