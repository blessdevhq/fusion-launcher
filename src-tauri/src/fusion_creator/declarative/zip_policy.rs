//! Bounded ZIP metadata policy, before zip::ZipArchive allocates its index.
//! Payload decoding/extraction remains exclusively in ArchiveTransformer.
use std::io::{Read, Seek, SeekFrom};

use super::{invalid, ZipNormalizationLimits};
use crate::fusion_source::errors::FusionError;

fn at<const N: usize>(input: &mut (impl Read + Seek), offset: u64) -> Result<[u8; N], FusionError> {
    input
        .seek(SeekFrom::Start(offset))
        .map_err(|_| invalid("Invalid ZIP metadata offset."))?;
    let mut bytes = [0; N];
    input
        .read_exact(&mut bytes)
        .map_err(|_| invalid("Truncated ZIP metadata."))?;
    Ok(bytes)
}
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("fixed ZIP header"),
    )
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed ZIP header"),
    )
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed ZIP header"),
    )
}

pub(super) fn preflight(
    input: &mut (impl Read + Seek),
    limits: ZipNormalizationLimits,
) -> Result<usize, FusionError> {
    let length = input
        .seek(SeekFrom::End(0))
        .map_err(|_| invalid("Cannot measure ZIP."))?;
    if length < 22 {
        return Err(invalid("Truncated ZIP end record."));
    }
    // The only allocation here is bounded independently of hostile header fields.
    let tail_len = length.min(22 + u64::from(u16::MAX)) as usize;
    let tail_start = length - tail_len as u64;
    input
        .seek(SeekFrom::Start(tail_start))
        .map_err(|_| invalid("Invalid ZIP tail."))?;
    let mut tail = vec![0; tail_len];
    input
        .read_exact(&mut tail)
        .map_err(|_| invalid("Truncated ZIP tail."))?;
    let end = (0..=tail.len() - 22)
        .rev()
        .find(|offset| {
            tail[*offset..*offset + 4] == *b"PK\x05\x06"
                && *offset + 22 + usize::from(u16_at(&tail, *offset + 20)) == tail.len()
        })
        .ok_or_else(|| invalid("ZIP end record is missing or ambiguous."))?;
    let header = &tail[end..end + 22];
    let end_offset = tail_start + end as u64;
    if u16_at(header, 4) != 0 || u16_at(header, 6) != 0 {
        return Err(invalid("Multi-volume ZIPs are not supported."));
    }
    let mut count = u64::from(u16_at(header, 10));
    let mut size = u64::from(u32_at(header, 12));
    let mut start = u64::from(u32_at(header, 16));
    let budget = (limits.max_entries as u64)
        .saturating_mul(4096)
        .min(32 << 20);
    let sentinel = count == u64::from(u16::MAX)
        || u16_at(header, 8) == u16::MAX
        || size == u64::from(u32::MAX)
        || start == u64::from(u32::MAX);
    let locator = if end_offset >= 20 {
        Some(at::<20>(input, end_offset - 20)?)
    } else {
        None
    };
    let has_locator = locator.as_ref().is_some_and(|v| &v[..4] == b"PK\x06\x07");
    let mut zip64_start = None;
    let directory_end = if sentinel || has_locator {
        let locator = locator
            .filter(|v| &v[..4] == b"PK\x06\x07")
            .ok_or_else(|| invalid("ZIP64 locator is missing."))?;
        if u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
            return Err(invalid("Multi-volume ZIP64 is not supported."));
        }
        let record_start = u64_at(&locator, 8);
        zip64_start = Some(record_start);
        let record = at::<56>(input, record_start)?;
        let record_len = u64_at(&record, 4);
        if &record[..4] != b"PK\x06\x06"
            || record_len < 44
            || record_len > budget
            || record_start
                .checked_add(12)
                .and_then(|n| n.checked_add(record_len))
                != Some(end_offset - 20)
            || u32_at(&record, 16) != 0
            || u32_at(&record, 20) != 0
            || u64_at(&record, 24) != u64_at(&record, 32)
        {
            return Err(invalid("Invalid or oversized ZIP64 end record."));
        }
        let total = u64_at(&record, 32);
        let bytes = u64_at(&record, 40);
        let offset = u64_at(&record, 48);
        if (count != u64::from(u16::MAX) && count != total)
            || (u16_at(header, 8) != u16::MAX && u64::from(u16_at(header, 8)) != total)
            || (size != u64::from(u32::MAX) && size != bytes)
            || (start != u64::from(u32::MAX) && start != offset)
        {
            return Err(invalid("ZIP64 and classic end records disagree."));
        }
        count = total;
        size = bytes;
        start = offset;
        record_start
    } else {
        if u64::from(u16_at(header, 8)) != count {
            return Err(invalid("ZIP entry counts disagree."));
        }
        end_offset
    };
    if count > limits.max_entries as u64 {
        return Err(invalid("ZIP entry limit before indexing."));
    }
    if size > budget {
        return Err(invalid("ZIP metadata budget before indexing."));
    }
    if start.checked_add(size) != Some(directory_end)
        || count.checked_mul(46).is_none_or(|minimum| minimum > size)
    {
        return Err(invalid("Invalid ZIP central directory extent."));
    }
    // Validate every raw header before the library can allocate from its count.
    let mut cursor = start;
    let mut actual = 0u64;
    while cursor < directory_end {
        if directory_end - cursor < 46 {
            return Err(invalid("Truncated ZIP central header."));
        }
        let entry = at::<46>(input, cursor)?;
        if &entry[..4] != b"PK\x01\x02" {
            return Err(invalid("Invalid ZIP central header."));
        }
        actual += 1;
        if actual > count {
            return Err(invalid("ZIP raw and advertised entry counts disagree."));
        }
        let variable = u64::from(u16_at(&entry, 28))
            + u64::from(u16_at(&entry, 30))
            + u64::from(u16_at(&entry, 32));
        cursor = cursor
            .checked_add(46)
            .and_then(|n| n.checked_add(variable))
            .filter(|end| *end <= directory_end)
            .ok_or_else(|| invalid("ZIP central header exceeds metadata extent."))?;
    }
    if actual != count {
        return Err(invalid("ZIP raw and advertised entry counts disagree."));
    }
    // Match the pinned parser's interpretation before it sees the real file.
    // It accepts trailing garbage and backtracks into arbitrary earlier EOCDs.
    // Expose only bounded metadata (at original virtual offsets) to the probe;
    // payload signatures are hidden and a failed index cannot fall back to them.
    let metadata_len = length
        .checked_sub(start)
        .filter(|bytes| *bytes <= budget.saturating_mul(2).saturating_add(65_577))
        .ok_or_else(|| invalid("ZIP metadata extent exceeds the host budget."))?;
    let mut metadata = vec![0; metadata_len as usize];
    input
        .seek(SeekFrom::Start(start))
        .map_err(|_| invalid("Invalid ZIP metadata extent."))?;
    input
        .read_exact(&mut metadata)
        .map_err(|_| invalid("Truncated ZIP metadata extent."))?;
    for (offset, magic) in metadata.windows(4).enumerate() {
        let position = start + offset as u64;
        let expected = match magic {
            b"PK\x05\x06" => position == end_offset,
            b"PK\x06\x06" => Some(position) == zip64_start,
            b"PK\x06\x07" => zip64_start.is_some() && position == end_offset - 20,
            _ => true,
        };
        if !expected {
            return Err(invalid("Ambiguous ZIP metadata end records."));
        }
    }
    let probe = zip::ZipArchive::new(MetadataView {
        input: &mut *input,
        metadata,
        start,
        position: 0,
    })
    .map_err(|error| {
        invalid("ZIP metadata parser rejected the verified interpretation.")
            .with_detail("reason", error.to_string())
    })?;
    if probe.len() != count as usize || probe.central_directory_start() != start {
        return Err(invalid(
            "ZIP contains duplicate names or a different metadata interpretation.",
        ));
    }
    Ok(count as usize)
}

/// A sparse metadata-only view, not an extractor or payload buffer. Returning an
/// error for reads wholly below metadata stops the library's fallback search.
struct MetadataView<'a, R> {
    input: &'a mut R,
    metadata: Vec<u8>,
    start: u64,
    position: u64,
}
impl<R: Read + Seek> Read for MetadataView<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = self.start + self.metadata.len() as u64;
        if output.is_empty() || self.position >= length {
            return Ok(0);
        }
        let count = (length - self.position).min(output.len() as u64) as usize;
        let end = self.position + count as u64;
        if end <= self.start {
            // zip 2.4.2 validates each member's fixed 30-byte local header while
            // indexing. Permit only that bounded record, never payload scanning
            // or a second end-record interpretation hidden inside this header.
            if count == 30 {
                let header = at::<30>(self.input, self.position).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.message)
                })?;
                if &header[..4] == b"PK\x03\x04"
                    && !header
                        .windows(4)
                        .any(|magic| matches!(magic, b"PK\x05\x06" | b"PK\x06\x06" | b"PK\x06\x07"))
                {
                    output[..30].copy_from_slice(&header);
                    self.position = end;
                    return Ok(30);
                }
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "ZIP index attempted payload fallback at {} with {} bytes (metadata starts {})",
                    self.position, count, self.start
                ),
            ));
        }
        let zeroes = self.start.saturating_sub(self.position).min(count as u64) as usize;
        output[..zeroes].fill(0);
        let source = (self.position + zeroes as u64 - self.start) as usize;
        output[zeroes..count].copy_from_slice(&self.metadata[source..source + count - zeroes]);
        self.position = end;
        Ok(count)
    }
}
impl<R> Seek for MetadataView<'_, R> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::End(delta) => {
                (self.start + self.metadata.len() as u64).checked_add_signed(delta)
            }
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        }
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "ZIP seek overflow")
        })?;
        Ok(self.position)
    }
}
