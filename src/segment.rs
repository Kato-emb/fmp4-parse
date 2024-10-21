use std::{
    fmt,
    io::{Read, Seek, Write},
};

use mp4::{
    stsc::StscEntry, stts::SttsEntry, BoxHeader, BoxType, FtypBox, MoofBox, MoovBox, Mp4Box,
    ReadBox, StypBox, WriteBox,
};
use serde::Serialize;

use crate::{error::Fmp4ParseError, Result};

pub trait Segment: Sized {
    fn read<R: Read + Seek>(reader: &mut R) -> Result<Self>;
    fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct InitialSegment {
    pub ftyp: FtypBox,
    pub moov: MoovBox,
}

impl Segment for InitialSegment {
    fn read<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        let mut data = Self::default();

        while let Ok(header) = BoxHeader::read(reader) {
            match header.name {
                BoxType::FtypBox => {
                    data.ftyp = FtypBox::read_box(reader, header.size)?;
                }
                BoxType::MoovBox => {
                    data.moov = MoovBox::read_box(reader, header.size)?;
                }
                _ => {
                    mp4::skip_box(reader, header.size)?;
                }
            }
        }

        if data.moov.mvex.is_none() {
            Err(Fmp4ParseError::InvalidFormat(
                "Fmp4 initial segment must be set MvexBox.",
            ))
        } else {
            Ok(data)
        }
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.ftyp.write_box(writer)?;
        self.moov.write_box(writer)?;

        Ok(())
    }
}

impl fmt::Display for InitialSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[ftyp] size=8+{}\n[moov] size=8+{}",
            self.ftyp.box_size() - 8,
            self.moov.box_size() - 8
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Chunk {
    pub moof: MoofBox,
    pub mdat: Vec<u8>,
}

impl Chunk {
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.moof.write_box(writer)?;

        BoxHeader::new(BoxType::MdatBox, mp4::HEADER_SIZE + self.mdat.len() as u64)
            .write(writer)?;
        writer.write_all(&self.mdat)?;

        Ok(())
    }
}

impl fmt::Display for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[moof] size=8+{}\n[mdat] size=8+{}",
            self.moof.box_size() - 8,
            self.mdat.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct MediaSegment {
    pub styp: StypBox,
    pub chunks: Vec<Chunk>,
}

impl MediaSegment {
    /// Time-to-sample table
    pub fn stts_entries(&self, track_id: u32, default: u32) -> Vec<SttsEntry> {
        let mut entries: Vec<SttsEntry> = Vec::new();

        for chunk in self.chunks.iter() {
            let mut stts_entry = SttsEntry::default();

            if let Some(traf) = chunk
                .moof
                .trafs
                .iter()
                .find(|traf| traf.tfhd.track_id == track_id)
            {
                stts_entry.sample_count = traf
                    .trun
                    .as_ref()
                    .and_then(|trun| Some(trun.sample_count))
                    .unwrap_or(0);
                stts_entry.sample_delta = traf.tfhd.default_sample_duration.unwrap_or(default);
            }

            entries.push(stts_entry);
        }

        entries
    }

    /// Sample-to-chunk table
    pub fn stsc_entries(
        &self,
        track_id: u32,
        default: u32,
        chunk_offset: Option<u32>,
        sample_offset: Option<u32>,
    ) -> (Vec<StscEntry>, u32, u32) {
        let mut entries: Vec<StscEntry> = Vec::new();
        let mut chunk_count = chunk_offset.unwrap_or(1);
        let mut sample_count = sample_offset.unwrap_or(1);

        for chunk in self.chunks.iter() {
            if let Some(traf) = chunk
                .moof
                .trafs
                .iter()
                .find(|traf| traf.tfhd.track_id == track_id)
            {
                let trun = traf.trun.as_ref().unwrap();
                if !entries.last().is_some_and(|entry| {
                    entry.samples_per_chunk == trun.sample_count
                        && entry.sample_description_index
                            == traf.tfhd.sample_description_index.unwrap_or(default)
                }) {
                    let mut stsc_entry = StscEntry::default();

                    stsc_entry.sample_description_index =
                        traf.tfhd.sample_description_index.unwrap_or(default);
                    stsc_entry.first_chunk = chunk_count;
                    stsc_entry.samples_per_chunk = trun.sample_count;
                    stsc_entry.first_sample = sample_count;

                    entries.push(stsc_entry);
                }

                chunk_count += 1;
                sample_count += trun.sample_count;
            }
        }

        (entries, chunk_count, sample_count)
    }

    /// Sample size table
    pub fn stsz_entries(&self, track_id: u32, default: u32) -> Vec<u32> {
        let mut entries: Vec<u32> = Vec::new();

        for chunk in self.chunks.iter() {
            if let Some(traf) = chunk
                .moof
                .trafs
                .iter()
                .find(|traf| traf.tfhd.track_id == track_id)
            {
                let trun = traf.trun.as_ref().unwrap();

                if !trun.sample_sizes.is_empty() {
                    entries.extend(trun.sample_sizes.as_slice());
                } else {
                    let sample_size = traf.tfhd.default_sample_size.unwrap_or(default);
                    entries.extend(vec![sample_size; trun.sample_count as usize]);
                }
            }
        }

        entries
    }

    pub fn get_size(&self) -> u64 {
        self.chunks
            .iter()
            .map(|chunk| chunk.moof.box_size() + mp4::HEADER_SIZE + chunk.mdat.len() as u64)
            .sum()
    }
}

impl Segment for MediaSegment {
    fn read<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        let mut media = MediaSegment::default();

        while let Ok(header) = BoxHeader::read(reader) {
            match header.name {
                BoxType::StypBox => {
                    media.styp = StypBox::read_box(reader, header.size)?;
                }
                BoxType::MoofBox => {
                    let mut chunk = Chunk::default();
                    chunk.moof = MoofBox::read_box(reader, header.size)?;

                    let header = BoxHeader::read(reader)?;

                    if header.name != BoxType::MdatBox {
                        return Err(Fmp4ParseError::InvalidFormat(
                            "MdatBox should be after MoofBox in the media segment",
                        ));
                    }

                    let mut mdat = vec![0u8; header.size as usize - 8];
                    reader.read_exact(&mut mdat)?;
                    chunk.mdat = mdat;

                    media.chunks.push(chunk);
                }
                _ => {
                    mp4::skip_box(reader, header.size)?;
                }
            }
        }

        Ok(media)
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        self.styp.write_box(writer)?;

        for chunk in self.chunks.iter() {
            chunk.write(writer)?;
        }

        Ok(())
    }
}

impl fmt::Display for MediaSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut chunks = Vec::new();

        for chunk in self.chunks.iter() {
            chunks.push(format!("{chunk}"));
        }

        write!(
            f,
            "[styp] size=8+{}\n{}",
            self.styp.box_size() - 8,
            chunks.join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::PathBuf};

    use super::*;

    #[test]
    fn test_segment_initial_file_parse() {
        let path = PathBuf::from("resources/init.cmfi");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let init = InitialSegment::read(&mut reader).expect("Failed to parse initial data");
        println!("{init}");

        let mut copy_path = path.clone();
        copy_path.set_extension("copy");
        let mut file = std::fs::File::create(&copy_path).unwrap();
        assert!(init.write(&mut file).is_ok());

        let data = std::fs::read(&copy_path).unwrap();
        let mut reader = Cursor::new(data);
        let copy_init = InitialSegment::read(&mut reader).expect("Failed to parse initial data");
        println!("{copy_init}");
        assert_eq!(init, copy_init);

        std::fs::remove_file(copy_path).unwrap();
    }

    #[test]
    fn test_segment_media_file_parse() {
        let path = PathBuf::from("resources/media.cmfv");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let media = MediaSegment::read(&mut reader).expect("Failed to parse fragmented media data");

        let entries = media.stts_entries(1, 0);
        println!("{:?}", entries);
        println!("{:?}", media.stsc_entries(1, 1, None, None));
        let stsz = media.stsz_entries(1, 0);
        println!("{:?}", stsz.len());

        let mut copy_path = path.clone();
        copy_path.set_extension("copy");
        let mut file = std::fs::File::create(&copy_path).unwrap();
        assert!(media.write(&mut file).is_ok());

        let data = std::fs::read(&copy_path).unwrap();
        let mut reader = Cursor::new(data);
        let copy_media =
            MediaSegment::read(&mut reader).expect("Failed to parse fragmented media data");

        assert_eq!(media, copy_media);
        println!("{:#?}", media.chunks[0].moof);

        std::fs::remove_file(copy_path).unwrap();
    }
}
