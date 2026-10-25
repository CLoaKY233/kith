//! Minimal RFC 1035 wire reading and writing.
//!
//! Every read and write is bounds-checked, every offset computed with checked
//! arithmetic, and nothing allocates. The writer never writes past its limit: a write
//! that does not fit returns [`Overflow`] and leaves the buffer as it was before.

/// Most compression targets remembered per message. More names still encode correctly,
/// just less compactly.
const MAX_COMPRESSION_TARGETS: usize = 64;
/// Compression pointers can address only the first 16 KiB of a message.
const MAX_POINTER_OFFSET: usize = 0x3FFF;
/// Pointer hops followed while comparing names, which stops loops.
const MAX_POINTER_HOPS: usize = 16;
/// Longest name on the wire (RFC 1035 §2.3.4).
pub(crate) const MAX_WIRE_NAME_LEN: usize = 255;

/// Byte that stands in for a query-name byte no zone name can contain (including a
/// literal `.` inside a label), so such names keep their label boundaries and never
/// match a zone name.
const FOREIGN_BYTE: u8 = 0x01;

/// The output does not have room for the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overflow;

/// The input is not a well-formed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Malformed;

/// Cursor over received bytes.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8], position: usize) -> Self {
        Self { bytes, position }
    }

    pub(crate) fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], Malformed> {
        let end = self.position.checked_add(count).ok_or(Malformed)?;
        let taken = self.bytes.get(self.position..end).ok_or(Malformed)?;
        self.position = end;
        Ok(taken)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Malformed> {
        let [byte] = self.take(1)? else {
            return Err(Malformed);
        };
        Ok(*byte)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, Malformed> {
        let [high, low] = self.take(2)? else {
            return Err(Malformed);
        };
        Ok(u16::from_be_bytes([*high, *low]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Malformed> {
        let [a, b, c, d] = self.take(4)? else {
            return Err(Malformed);
        };
        Ok(u32::from_be_bytes([*a, *b, *c, *d]))
    }

    pub(crate) fn skip(&mut self, count: usize) -> Result<(), Malformed> {
        self.take(count).map(|_| ())
    }

    /// Reads an uncompressed name into `name` as lowercase dotted text.
    pub(crate) fn question_name(&mut self, name: &mut NameText) -> Result<(), Malformed> {
        let start = self.position;
        loop {
            let length = self.u8()?;
            if length == 0 {
                break;
            }
            // Pointers (0b11) and the obsolete extended label types are rejected: a
            // question is the first name, so nothing exists for a pointer to reach.
            if length & 0xC0 != 0 {
                return Err(Malformed);
            }
            let label = self.take(usize::from(length))?;
            name.push_label(label)?;
        }
        if self.position.checked_sub(start).ok_or(Malformed)? > MAX_WIRE_NAME_LEN {
            return Err(Malformed);
        }
        Ok(())
    }

    /// Skips a possibly compressed name, returning whether it was the root.
    pub(crate) fn skip_name(&mut self) -> Result<bool, Malformed> {
        let mut root = true;
        for _ in 0..=MAX_WIRE_NAME_LEN {
            let length = self.u8()?;
            match length & 0xC0 {
                0 if length == 0 => return Ok(root),
                0 => self.skip(usize::from(length))?,
                0xC0 => {
                    self.skip(1)?;
                    return Ok(false);
                }
                _ => return Err(Malformed),
            }
            root = false;
        }
        Err(Malformed)
    }
}

/// A query name as lowercase dotted text in a fixed buffer.
#[derive(Debug)]
pub(crate) struct NameText {
    bytes: [u8; MAX_WIRE_NAME_LEN],
    length: usize,
}

impl NameText {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: [0; MAX_WIRE_NAME_LEN],
            length: 0,
        }
    }

    fn push(&mut self, byte: u8) -> Result<(), Malformed> {
        let slot = self.bytes.get_mut(self.length).ok_or(Malformed)?;
        *slot = byte;
        self.length = self.length.checked_add(1).ok_or(Malformed)?;
        Ok(())
    }

    fn push_label(&mut self, label: &[u8]) -> Result<(), Malformed> {
        if self.length != 0 {
            self.push(b'.')?;
        }
        for &byte in label {
            let mapped = match byte.to_ascii_lowercase() {
                lower @ (b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'*') => lower,
                _ => FOREIGN_BYTE,
            };
            self.push(mapped)?;
        }
        Ok(())
    }

    pub(crate) fn as_str(&self) -> &str {
        // Every pushed byte is ASCII, so this never fails; an empty name is the fallback.
        self.bytes
            .get(..self.length)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
            .unwrap_or("")
    }
}

/// Bounded writer with name compression.
#[derive(Debug)]
pub(crate) struct Writer<'a> {
    buffer: &'a mut [u8],
    length: usize,
    limit: usize,
    targets: [u16; MAX_COMPRESSION_TARGETS],
    target_count: usize,
}

impl<'a> Writer<'a> {
    /// A writer that will use at most `limit` bytes of `buffer`.
    pub(crate) fn new(buffer: &'a mut [u8], limit: usize) -> Self {
        let limit = limit.min(buffer.len());
        Self {
            buffer,
            length: 0,
            limit,
            targets: [0; MAX_COMPRESSION_TARGETS],
            target_count: 0,
        }
    }

    pub(crate) fn position(&self) -> usize {
        self.length
    }

    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit.min(self.buffer.len());
    }

    /// Drops everything written from `position` on.
    pub(crate) fn truncate(&mut self, position: usize) {
        self.length = self.length.min(position);
        let mut kept = 0_usize;
        for index in 0..self.target_count {
            if let Some(&target) = self.targets.get(index)
                && usize::from(target) < self.length
                && let Some(slot) = self.targets.get_mut(kept)
            {
                *slot = target;
                kept = kept.saturating_add(1);
            }
        }
        self.target_count = kept;
    }

    pub(crate) fn bytes(&mut self, data: &[u8]) -> Result<(), Overflow> {
        let end = self.length.checked_add(data.len()).ok_or(Overflow)?;
        if end > self.limit {
            return Err(Overflow);
        }
        self.buffer
            .get_mut(self.length..end)
            .ok_or(Overflow)?
            .copy_from_slice(data);
        self.length = end;
        Ok(())
    }

    pub(crate) fn u8(&mut self, value: u8) -> Result<(), Overflow> {
        self.bytes(&[value])
    }

    pub(crate) fn u16(&mut self, value: u16) -> Result<(), Overflow> {
        self.bytes(&value.to_be_bytes())
    }

    pub(crate) fn u32(&mut self, value: u32) -> Result<(), Overflow> {
        self.bytes(&value.to_be_bytes())
    }

    /// Overwrites two already-written bytes at `at`.
    pub(crate) fn patch_u16(&mut self, at: usize, value: u16) -> Result<(), Overflow> {
        let end = at.checked_add(2).ok_or(Overflow)?;
        if end > self.length {
            return Err(Overflow);
        }
        self.buffer
            .get_mut(at..end)
            .ok_or(Overflow)?
            .copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Patches a byte already written at `at` with `update`.
    pub(crate) fn patch_u8(
        &mut self,
        at: usize,
        update: impl FnOnce(u8) -> u8,
    ) -> Result<(), Overflow> {
        if at >= self.length {
            return Err(Overflow);
        }
        let byte = self.buffer.get_mut(at).ok_or(Overflow)?;
        *byte = update(*byte);
        Ok(())
    }

    /// A compression pointer to `offset`.
    pub(crate) fn pointer(&mut self, offset: u16) -> Result<(), Overflow> {
        self.u16(0xC000 | offset)
    }

    /// Records each label of the name already written at `offset` as a compression target.
    pub(crate) fn remember_name_at(&mut self, mut offset: usize) {
        for _ in 0..MAX_WIRE_NAME_LEN {
            let Some(&length) = self.written().get(offset) else {
                return;
            };
            if length == 0 || length & 0xC0 != 0 {
                return;
            }
            self.remember(offset);
            let Some(next) = offset
                .checked_add(usize::from(length))
                .and_then(|o| o.checked_add(1))
            else {
                return;
            };
            offset = next;
        }
    }

    /// Writes a lowercase dotted name, compressing against earlier names.
    pub(crate) fn name(&mut self, name: &str) -> Result<(), Overflow> {
        let mut rest = name;
        while !rest.is_empty() {
            if let Some(offset) = self.find(rest) {
                return self.pointer(offset);
            }
            let (label, tail) = rest.split_once('.').unwrap_or((rest, ""));
            let here = self.length;
            self.label(label)?;
            self.remember(here);
            rest = tail;
        }
        self.u8(0)
    }

    /// Writes a name without compression (RFC 2782 forbids it for SRV targets).
    pub(crate) fn name_uncompressed(&mut self, name: &str) -> Result<(), Overflow> {
        for label in name.split('.').filter(|label| !label.is_empty()) {
            self.label(label)?;
        }
        self.u8(0)
    }

    pub(crate) fn label(&mut self, label: &str) -> Result<(), Overflow> {
        let length = u8::try_from(label.len()).map_err(|_| Overflow)?;
        self.u8(length)?;
        self.bytes(label.as_bytes())
    }

    fn written(&self) -> &[u8] {
        self.buffer.get(..self.length).unwrap_or(&[])
    }

    fn remember(&mut self, offset: usize) {
        let Ok(offset) = u16::try_from(offset) else {
            return;
        };
        if usize::from(offset) > MAX_POINTER_OFFSET {
            return;
        }
        if let Some(slot) = self.targets.get_mut(self.target_count) {
            *slot = offset;
            self.target_count = self.target_count.saturating_add(1);
        }
    }

    fn find(&self, suffix: &str) -> Option<u16> {
        self.targets
            .get(..self.target_count)?
            .iter()
            .copied()
            .find(|&offset| self.matches_at(usize::from(offset), suffix))
    }

    /// Whether the name written at `offset` equals `suffix`, ignoring ASCII case.
    fn matches_at(&self, mut offset: usize, suffix: &str) -> bool {
        let written = self.written();
        let mut labels = suffix.split('.');
        let mut hops = 0_usize;
        loop {
            let Some(&length) = written.get(offset) else {
                return false;
            };
            if length & 0xC0 == 0xC0 {
                let Some(&low) = offset.checked_add(1).and_then(|o| written.get(o)) else {
                    return false;
                };
                hops = hops.saturating_add(1);
                if hops > MAX_POINTER_HOPS {
                    return false;
                }
                offset = usize::from(u16::from_be_bytes([length & 0x3F, low]));
                continue;
            }
            if length & 0xC0 != 0 {
                return false;
            }
            let Some(label) = labels.next() else {
                return length == 0;
            };
            let Some(start) = offset.checked_add(1) else {
                return false;
            };
            let Some(end) = start.checked_add(usize::from(length)) else {
                return false;
            };
            match written.get(start..end) {
                Some(bytes)
                    if !label.is_empty() && bytes.eq_ignore_ascii_case(label.as_bytes()) =>
                {
                    offset = end;
                }
                _ => return false,
            }
        }
    }
}
