use std::io::{Seek, Write};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use mp4::{
    Avc1Box, BoxHeader, BoxType, FourCC, FtypBox, Hev1Box, MoovBox, Mp4aBox, MvexBox, StblBox,
    TrakBox, Tx3gBox, Vp09Box, WriteBox,
};

use crate::{error::Fmp4ParseError, segment::InitialSegment, Result};

#[derive(Debug)]
pub enum MediaBox {
    Avc1(Avc1Box),
    Hev1(Hev1Box),
    Vp9(Vp09Box),
    Mp4a(Mp4aBox),
    Tx3g(Tx3gBox),
    __Unknown,
}

impl From<&StblBox> for MediaBox {
    fn from(value: &StblBox) -> Self {
        if let Some(ref avc1) = value.stsd.avc1 {
            MediaBox::Avc1(avc1.clone())
        } else if let Some(ref hev1) = value.stsd.hev1 {
            MediaBox::Hev1(hev1.clone())
        } else if let Some(ref vp09) = value.stsd.vp09 {
            MediaBox::Vp9(vp09.clone())
        } else if let Some(ref mp4a) = value.stsd.mp4a {
            MediaBox::Mp4a(mp4a.clone())
        } else if let Some(ref tx3g) = value.stsd.tx3g {
            MediaBox::Tx3g(tx3g.clone())
        } else {
            MediaBox::__Unknown
        }
    }
}

#[derive(Debug)]
pub struct TrackExtendData {
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl From<&MvexBox> for TrackExtendData {
    fn from(value: &MvexBox) -> Self {
        Self {
            track_id: value.trex.track_id,
            default_sample_description_index: value.trex.default_sample_description_index,
            default_sample_duration: value.trex.default_sample_duration,
            default_sample_size: value.trex.default_sample_size,
            default_sample_flags: value.trex.default_sample_flags,
        }
    }
}

#[derive(Debug)]
pub struct TrackData {
    pub width: u16,
    pub height: u16,
    pub timescale: u32,
    pub media_box: MediaBox,
}

impl From<&TrakBox> for TrackData {
    fn from(value: &TrakBox) -> Self {
        Self {
            width: value.tkhd.width.value(),
            height: value.tkhd.height.value(),
            timescale: value.mdia.mdhd.timescale,
            media_box: MediaBox::from(&value.mdia.minf.stbl),
        }
    }
}

#[derive(Debug)]
pub struct FMp4Config {
    major_brand: FourCC,
    minor_version: u32,
    compatible_brands: Vec<FourCC>,
    tracks: Vec<(TrackData, TrackExtendData)>,
}

impl FMp4Config {
    pub fn add_track(&mut self, initial_segment: &InitialSegment) -> Result<()> {
        let extend_data = TrackExtendData::from(
            initial_segment
                .moov
                .mvex
                .as_ref()
                .ok_or(Fmp4ParseError::InvalidFormat("Missing mvex box"))?,
        );

        let Some(trak) = initial_segment
            .moov
            .traks
            .iter()
            .find(|trak| trak.tkhd.track_id == extend_data.track_id)
        else {
            return Err(Fmp4ParseError::InvalidFormat("Missing trak box"));
        };

        let track_data = TrackData::from(trak);
        self.tracks.push((track_data, extend_data));

        Ok(())
    }
}

#[derive(Debug)]
pub struct HybridMp4Writer<W> {
    writer: W,
    free_pos: u64,
    moov: MoovBox,
    // tracks: HashMap<u32, TrackExtends>,
}

impl<W: Write + Seek> HybridMp4Writer<W> {
    pub fn initialize(mut writer: W, config: &FMp4Config) -> Result<Self> {
        let ftyp = FtypBox {
            major_brand: config.major_brand,
            minor_version: config.minor_version,
            compatible_brands: config.compatible_brands.clone(),
        };
        ftyp.write_box(&mut writer)?;

        let free_pos = writer.stream_position()?;
        BoxHeader::new(BoxType::FreeBox, 0).write(&mut writer)?;

        let mut moov = MoovBox::default();
        moov.mvhd.version = 1;

        Ok(Self {
            writer,
            free_pos,
            moov,
            // tracks: HashMap::new(),
        })
    }

    pub fn finalize(&mut self) -> Result<()> {
        // Represent the calendar date and time in seconds since midnight, January 1, 1904, preferably using coordinated universal time (UTC).
        let epoch = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(1904, 1, 1).unwrap(),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        )
        .and_utc();

        let now = Utc::now();
        let creation_time = now.signed_duration_since(epoch).num_seconds() as u64;
        self.moov.mvhd.creation_time = creation_time;
        self.moov.mvhd.modification_time = creation_time;

        // while let Some(track_extend) = self.tracks.get(&self.moov.mvhd.next_track_id) {
        //     self.moov.mvhd.next_track_id += 1;
        // }

        println!("{:?}", self.moov);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writer_hybrid_mp4() {
        // let file = std::fs::File::create("resources/test.mp4").unwrap();
        // let mut writer = HybridMp4Writer::initialize(file, &FMp4Config::default()).unwrap();
        // writer.finalize().unwrap();
    }
}
