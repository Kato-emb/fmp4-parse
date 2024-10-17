use std::{
    io::{Seek, Write},
    path::Path,
};

use error::Fmp4ParseError;
use mp4::{
    stsc::StscEntry, stts::SttsEntry, BoxHeader, BoxType, Co64Box, MoovBox, StscBox, StszBox,
    SttsBox, WriteBox,
};
use segment::{InitialSegment, MediaSegment};

mod error;
mod segment;
mod writer;

pub type Result<T> = std::result::Result<T, Fmp4ParseError>;

#[derive(Debug)]
pub struct HybridMp4 {
    writer: std::fs::File,
    place_holder: u64,
    mdat_size: u64,
    chunk_offset: u32,
    sample_offset: u32,
    moov: MoovBox,
    stts_entries: Vec<SttsEntry>,
    stsc_entries: Vec<StscEntry>,
    stsz_entries: Vec<u32>,
    co64_entries: Vec<u64>,
}

impl HybridMp4 {
    pub fn new<P: AsRef<Path>>(path: P, mut init: InitialSegment) -> Result<Self> {
        let mut writer = std::fs::File::create(path)?;
        init.ftyp.major_brand = 0x69736F6D.into(); // isom
        init.ftyp.write_box(&mut writer)?;

        let place_holder = writer.stream_position()?;
        BoxHeader::new(BoxType::FreeBox, 0).write(&mut writer)?;

        let moov = init.moov;

        Ok(Self {
            writer,
            place_holder,
            mdat_size: 0,
            chunk_offset: 1,
            sample_offset: 1,
            moov,
            stts_entries: Vec::new(),
            stsc_entries: Vec::new(),
            stsz_entries: Vec::new(),
            co64_entries: Vec::new(),
        })
    }

    pub fn add_fragment(&mut self, media: MediaSegment) -> Result<()> {
        let mvex = self.moov.mvex.as_ref().unwrap();
        let default_idx = mvex.trex.default_sample_description_index;
        let track_id = mvex.trex.track_id;
        self.stts_entries.extend(media.stts_entries(track_id));

        let (stsc_entries, chunk_count, sample_count) = media.stsc_entries(
            track_id,
            default_idx,
            Some(self.chunk_offset),
            Some(self.sample_offset),
        );
        self.chunk_offset = chunk_count;
        self.sample_offset = sample_count;

        self.stsc_entries.extend(stsc_entries);

        self.stsz_entries.extend(media.stsz_entries(track_id));

        for chunk in media.chunks.iter() {
            chunk.moof.write_box(&mut self.writer)?;

            self.co64_entries
                .push(self.writer.stream_position()? + mp4::HEADER_SIZE);
            BoxHeader::new(BoxType::MdatBox, mp4::HEADER_SIZE + chunk.mdat.len() as u64)
                .write(&mut self.writer)?;
            self.writer.write_all(&chunk.mdat)?;
        }

        self.mdat_size += media.get_size();

        Ok(())
    }

    pub fn finalize(&mut self) -> Result<()> {
        let duration = self
            .stts_entries
            .iter()
            .map(|stts| stts.sample_count * stts.sample_delta)
            .sum::<u32>();
        self.moov.mvhd.duration = duration as u64;

        let track_id = self.moov.mvex.as_ref().unwrap().trex.track_id;

        if let Some(trak) = self
            .moov
            .traks
            .iter_mut()
            .find(|trak| trak.tkhd.track_id == track_id)
        {
            trak.tkhd.duration = duration as u64;
            trak.mdia.mdhd.duration = duration as u64;

            let mut stts = SttsBox::default();
            stts.entries.append(&mut self.stts_entries);
            trak.mdia.minf.stbl.stts = stts;

            let mut stsc = StscBox::default();
            stsc.entries.append(&mut self.stsc_entries);
            trak.mdia.minf.stbl.stsc = stsc;

            let mut stsz = StszBox::default();
            stsz.sample_count = self.stsz_entries.len() as u32;
            stsz.sample_sizes.append(&mut self.stsz_entries);
            trak.mdia.minf.stbl.stsz = stsz;

            let mut co64 = Co64Box::default();
            co64.entries.append(&mut self.co64_entries);
            trak.mdia.minf.stbl.co64 = Some(co64);

            // stco and co64 never exist at the same time.
            trak.mdia.minf.stbl.stco = None;
        }

        self.moov.write_box(&mut self.writer)?;

        self.writer
            .seek(std::io::SeekFrom::Start(self.place_holder))?;
        BoxHeader::new(BoxType::MdatBox, mp4::HEADER_SIZE + self.mdat_size)
            .write(&mut self.writer)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::PathBuf};

    use segment::Segment;

    use super::*;

    #[test]
    fn test_hybrid_mp4() {
        let path = PathBuf::from("resources/shiyakusyo/720p.cmfi");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let init = InitialSegment::read(&mut reader).expect("Failed to parse initial data");

        let mut writer = HybridMp4::new("resources/test.mp4", init).unwrap();

        let path = PathBuf::from("resources/shiyakusyo/10459.cmfv");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let media = MediaSegment::read(&mut reader).expect("Failed to parse fragmented media data");
        writer.add_fragment(media).unwrap();

        let path = PathBuf::from("resources/shiyakusyo/25459.cmfv");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let media = MediaSegment::read(&mut reader).expect("Failed to parse fragmented media data");
        writer.add_fragment(media).unwrap();

        let path = PathBuf::from("resources/shiyakusyo/40459.cmfv");
        let data = std::fs::read(&path).unwrap();
        let mut reader = Cursor::new(data);
        let media = MediaSegment::read(&mut reader).expect("Failed to parse fragmented media data");
        writer.add_fragment(media).unwrap();

        println!("{:?}", writer);
        writer.finalize().unwrap();
    }
}
