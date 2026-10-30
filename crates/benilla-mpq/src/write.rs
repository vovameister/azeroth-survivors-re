//! Writing a V1 archive in the shape this crate reads, for tools that pack a trimmed `Data`: 4 KiB
//! sectors, each zlib-compressed (codec byte `0x02`) unless that would not shrink it, the hash and
//! block tables encrypted with their usual keys, and a `(listfile)`. Files stream to disk as they
//! are added; only the tables wait for [`ArchiveWriter::finish`].

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use crate::crypto::{encrypt_block, hash_string, hash_type};
use crate::{Error, Result, FLAG_COMPRESS, FLAG_DELETE_MARKER, FLAG_EXISTS, SIGNATURE};

/// `512 << 3`: the 4 KiB sectors the 1.12.1 archives use.
const SECTOR_SHIFT: u16 = 3;
const SECTOR_SIZE: usize = 512 << SECTOR_SHIFT;
const HEADER_SIZE: u32 = 32;

/// An archive being written: [`add`](Self::add) each file, then [`finish`](Self::finish).
pub struct ArchiveWriter {
    out: BufWriter<File>,
    /// Where the next file's data goes, from the archive's start.
    pos: u64,
    /// `(name, block)` per file, in the order added.
    entries: Vec<(String, [u32; 4])>,
    /// Case- and separator-folded names already added, as the hash compares them.
    seen: std::collections::HashSet<String>,
}

impl ArchiveWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        // The header is written last, once the tables' places are known.
        out.write_all(&[0u8; HEADER_SIZE as usize])?;
        Ok(Self {
            out,
            pos: u64::from(HEADER_SIZE),
            entries: Vec::new(),
            seen: std::collections::HashSet::new(),
        })
    }

    /// Add `data` as `name` (`/` or `\`); a name already added is skipped, as one hash slot holds
    /// one file.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        if !self.seen.insert(name.replace('/', "\\").to_ascii_lowercase()) {
            return Ok(());
        }
        let file_pos = self.offset()?;
        let stored = if data.is_empty() {
            self.entries
                .push((name.to_owned(), [file_pos, 0, 0, FLAG_EXISTS]));
            return Ok(());
        } else {
            encode_sectors(data)
        };
        self.out.write_all(&stored)?;
        self.pos += stored.len() as u64;
        let size = u32::try_from(data.len())
            .map_err(|_| Error::Corrupt(format!("{name}: larger than 4 GiB")))?;
        self.entries.push((
            name.to_owned(),
            [file_pos, stored.len() as u32, size, FLAG_EXISTS | FLAG_COMPRESS],
        ));
        Ok(())
    }

    /// Add `name` as a delete marker: a chain walker stops at it, so a lower archive's copy is not
    /// read.
    pub fn delete(&mut self, name: &str) -> Result<()> {
        if !self.seen.insert(name.replace('/', "\\").to_ascii_lowercase()) {
            return Ok(());
        }
        let file_pos = self.offset()?;
        self.entries.push((
            name.to_owned(),
            [file_pos, 0, 0, FLAG_EXISTS | FLAG_DELETE_MARKER],
        ));
        Ok(())
    }

    /// Write the `(listfile)`, the tables and the header.
    pub fn finish(mut self) -> Result<()> {
        let listfile: String = self
            .entries
            .iter()
            .map(|(name, _)| format!("{name}\r\n"))
            .collect();
        self.add("(listfile)", listfile.as_bytes())?;

        // Twice the files, a power of two: the reader masks the slot with `len - 1`.
        let slots = (self.entries.len() * 2).next_power_of_two().max(4);
        let mut hash = vec![0xFFFF_FFFFu32; slots * 4];
        for (index, (name, _)) in self.entries.iter().enumerate() {
            let mut slot = hash_string(name, hash_type::TABLE_OFFSET) as usize & (slots - 1);
            while hash[slot * 4 + 3] != 0xFFFF_FFFF {
                slot = (slot + 1) & (slots - 1);
            }
            hash[slot * 4] = hash_string(name, hash_type::NAME_A);
            hash[slot * 4 + 1] = hash_string(name, hash_type::NAME_B);
            hash[slot * 4 + 2] = 0; // locale | platform: neutral
            hash[slot * 4 + 3] = index as u32;
        }
        encrypt_block(&mut hash, hash_string("(hash table)", hash_type::FILE_KEY));
        let mut block: Vec<u32> = self.entries.iter().flat_map(|(_, b)| *b).collect();
        encrypt_block(&mut block, hash_string("(block table)", hash_type::FILE_KEY));

        let hash_pos = self.offset()?;
        let block_pos = hash_pos + (hash.len() * 4) as u32;
        for w in hash.iter().chain(&block) {
            self.out.write_all(&w.to_le_bytes())?;
        }
        let end = u64::from(block_pos) + (block.len() * 4) as u64;
        let archive_size = u32::try_from(end)
            .map_err(|_| Error::Corrupt("archive larger than 4 GiB".into()))?;

        let mut header = [0u8; HEADER_SIZE as usize];
        header[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        header[4..8].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        header[8..12].copy_from_slice(&archive_size.to_le_bytes());
        // Bytes 12..14: format version 0, V1.
        header[14..16].copy_from_slice(&SECTOR_SHIFT.to_le_bytes());
        header[16..20].copy_from_slice(&hash_pos.to_le_bytes());
        header[20..24].copy_from_slice(&block_pos.to_le_bytes());
        header[24..28].copy_from_slice(&(slots as u32).to_le_bytes());
        header[28..32].copy_from_slice(&(self.entries.len() as u32).to_le_bytes());
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&header)?;
        self.out.flush()?;
        Ok(())
    }

    /// The next write's place as the tables store it, refused past what a `u32` can point at.
    fn offset(&self) -> Result<u32> {
        u32::try_from(self.pos).map_err(|_| Error::Corrupt("archive larger than 4 GiB".into()))
    }
}

/// A file as a sectored `COMPRESS` entry stores it: `sectors + 1` offsets from the entry's start,
/// then each sector, zlib with its codec byte where that is smaller, else verbatim (the reader
/// takes a sector as long as its share for stored).
fn encode_sectors(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    let chunks: Vec<&[u8]> = data.chunks(SECTOR_SIZE).collect();
    let mut body = Vec::with_capacity(data.len() / 2);
    let mut offsets = Vec::with_capacity(chunks.len() + 1);
    let table = (chunks.len() + 1) * 4;
    for chunk in &chunks {
        offsets.push((table + body.len()) as u32);
        let mut z = ZlibEncoder::new(vec![0x02u8], Compression::best());
        // Writing to a Vec cannot fail.
        let _ = z.write_all(chunk);
        let packed = z.finish().unwrap_or_default();
        if !packed.is_empty() && packed.len() < chunk.len() {
            body.extend_from_slice(&packed);
        } else {
            body.extend_from_slice(chunk);
        }
    }
    offsets.push((table + body.len()) as u32);
    let mut out = Vec::with_capacity(table + body.len());
    for o in offsets {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Archive;

    #[test]
    fn written_files_read_back_byte_for_byte() {
        let name = format!("benilla_mpq_write_{}.mpq", std::process::id());
        let path = std::env::temp_dir().join(name);
        let compressible = vec![7u8; 3 * SECTOR_SIZE + 100];
        // Bytes zlib cannot shrink: every sector is stored verbatim.
        let noise: Vec<u8> = (0..2 * SECTOR_SIZE as u32 + 17)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let mut w = ArchiveWriter::create(&path).unwrap();
        w.add("World\\a.adt", &compressible).unwrap();
        w.add("Sound/b.wav", &noise).unwrap();
        w.add("c.txt", b"").unwrap();
        w.add("WORLD/A.ADT", b"a duplicate name is skipped").unwrap();
        w.delete("Old\\gone.blp").unwrap();
        w.finish().unwrap();

        let a = Archive::open(&path).unwrap();
        assert_eq!(a.read_file("world/a.adt").unwrap(), compressible);
        assert_eq!(a.read_file("Sound\\b.wav").unwrap(), noise);
        assert_eq!(a.read_file("c.txt").unwrap(), b"");
        assert!(a.is_delete_marker("old/gone.blp"));
        assert!(!a.is_delete_marker("c.txt"));
        let list = String::from_utf8(a.read_file("(listfile)").unwrap()).unwrap();
        assert_eq!(list, "World\\a.adt\r\nSound/b.wav\r\nc.txt\r\nOld\\gone.blp\r\n");
        let _ = std::fs::remove_file(path);
    }
}
