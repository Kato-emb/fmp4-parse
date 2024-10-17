use std::io::{Seek, Write};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use mp4::{BoxHeader, BoxType, FourCC, FtypBox, MoovBox, WriteBox};

use crate::Result;

#[derive(Debug, Default)]
pub struct FMp4Config {
    pub major_brand: FourCC,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCC>,
    pub timescale: u32,
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

#[derive(Debug)]
pub struct HybridMp4Writer<W> {
    writer: W,
    free_pos: u64,
    moov: MoovBox,
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

        println!("{:?}", self.moov);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writer_hybrid_mp4() {
        let file = std::fs::File::create("resources/test.mp4").unwrap();
        let mut writer = HybridMp4Writer::initialize(file, &FMp4Config::default()).unwrap();
        writer.finalize().unwrap();
    }
}
