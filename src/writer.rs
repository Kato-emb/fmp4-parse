use std::{
    collections::HashMap,
    io::{Seek, Write},
    str::FromStr,
};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use mp4::*;
use stsc::StscEntry;
use stts::SttsEntry;

use crate::{
    error::Fmp4ParseError,
    segment::{InitialSegment, MediaSegment},
    Result,
};

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct TrackBaseData {
    pub width: u16,
    pub height: u16,
    pub timescale: u32,
    pub media_box: MediaBox,
}

impl From<&TrakBox> for TrackBaseData {
    fn from(value: &TrakBox) -> Self {
        Self {
            width: value.tkhd.width.value(),
            height: value.tkhd.height.value(),
            timescale: value.mdia.mdhd.timescale,
            media_box: MediaBox::from(&value.mdia.minf.stbl),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrackExtendData {
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub _default_sample_flags: u32,
}

impl From<&MvexBox> for TrackExtendData {
    fn from(value: &MvexBox) -> Self {
        Self {
            track_id: value.trex.track_id,
            default_sample_description_index: value.trex.default_sample_description_index,
            default_sample_duration: value.trex.default_sample_duration,
            default_sample_size: value.trex.default_sample_size,
            _default_sample_flags: value.trex.default_sample_flags,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrackData {
    pub base: TrackBaseData,
    pub extend: TrackExtendData,
}

impl TryFrom<&MoovBox> for TrackData {
    type Error = Fmp4ParseError;

    fn try_from(value: &MoovBox) -> Result<Self> {
        let extend = TrackExtendData::from(
            value
                .mvex
                .as_ref()
                .ok_or(Fmp4ParseError::InvalidFormat("Missing mvex box"))?,
        );

        let Some(trak) = value
            .traks
            .iter()
            .find(|trak| trak.tkhd.track_id == extend.track_id)
        else {
            return Err(Fmp4ParseError::InvalidFormat("Missing trak box"));
        };

        let base = TrackBaseData::from(trak);

        Ok(Self { base, extend })
    }
}

#[derive(Debug, Default)]
pub struct FMp4Config {
    pub major_brand: FourCC,
    pub minor_version: u32,
    pub compatible_brands: Vec<FourCC>,
    tracks: HashMap<u32, TrackData>,
}

impl FMp4Config {
    pub fn add_track(&mut self, initial_segment: &InitialSegment) -> Result<()> {
        let track_data = TrackData::try_from(&initial_segment.moov)?;
        self.tracks.insert(track_data.extend.track_id, track_data);

        Ok(())
    }
}

#[derive(Debug)]
pub struct Track {
    data: TrackData,
    stts_entries: Vec<SttsEntry>,
    stsc_entries: Vec<StscEntry>,
    stsz_entries: Vec<u32>,
    co64_entries: Vec<u64>,
    chunk_offset: u32,
    sample_offset: u32,
}

#[derive(Debug)]
pub struct HybridMp4Writer<W> {
    writer: W,
    free_pos: u64,
    free_size: u64,
    tracks: HashMap<u32, Track>,
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

        let mut tracks = HashMap::new();

        for (track_id, data) in config.tracks.iter() {
            tracks.insert(
                *track_id,
                Track {
                    data: data.clone(),
                    stts_entries: Vec::new(),
                    stsc_entries: Vec::new(),
                    stsz_entries: Vec::new(),
                    co64_entries: Vec::new(),
                    chunk_offset: 1,
                    sample_offset: 1,
                },
            );
        }

        Ok(Self {
            writer,
            free_pos,
            free_size: 0,
            tracks,
        })
    }

    pub fn add_fragment(&mut self, media: MediaSegment) -> Result<()> {
        for (track_id, track) in self.tracks.iter_mut() {
            track
                .stts_entries
                .extend(media.stts_entries(*track_id, track.data.extend.default_sample_duration));

            let (stsc_entries, chunk_count, sample_count) = media.stsc_entries(
                *track_id,
                track.data.extend.default_sample_description_index,
                Some(track.chunk_offset),
                Some(track.sample_offset),
            );
            track.chunk_offset = chunk_count;
            track.sample_offset = sample_count;
            track.stsc_entries.extend(stsc_entries);

            track
                .stsz_entries
                .extend(media.stsz_entries(*track_id, track.data.extend.default_sample_size));

            for chunk in media.chunks.iter() {
                chunk.moof.write_box(&mut self.writer)?;

                let offset = self.writer.stream_position()? + HEADER_SIZE;
                BoxHeader::new(BoxType::MdatBox, HEADER_SIZE + chunk.mdat.len() as u64)
                    .write(&mut self.writer)?;
                if self.writer.write_all(&chunk.mdat).is_ok() {
                    track.co64_entries.push(offset);
                }
            }

            self.free_size += media.get_size();
        }

        Ok(())
    }

    pub fn finalize(mut self) -> Result<()> {
        let mut moov = MoovBox::default();
        moov.mvhd.version = 1;

        // Represent the calendar date and time in seconds since midnight, January 1, 1904, preferably using coordinated universal time (UTC).
        let epoch = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(1904, 1, 1).unwrap(),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        )
        .and_utc();

        let now = Utc::now();
        let creation_time = now.signed_duration_since(epoch).num_seconds() as u64;
        moov.mvhd.creation_time = creation_time;
        moov.mvhd.modification_time = creation_time;

        let mut moov_duration = 0;
        while let Some(track) = self.tracks.get_mut(&moov.mvhd.next_track_id) {
            let mut trak = TrakBox::default();
            trak.tkhd.track_id = track.data.extend.track_id;
            trak.tkhd.set_width(track.data.base.width);
            trak.tkhd.set_height(track.data.base.height);
            let duration = track
                .stts_entries
                .iter()
                .map(|stts| (stts.sample_count * stts.sample_delta) as u64)
                .sum::<u64>();
            trak.tkhd.duration = duration;
            trak.mdia.mdhd.duration = duration;
            trak.mdia.mdhd.timescale = track.data.base.timescale;
            // trak.mdia.minf.vmhd = Some(VmhdBox::default());

            match &track.data.base.media_box {
                MediaBox::Avc1(avc1) => {
                    trak.mdia.minf.stbl.stsd.avc1 = Some(avc1.clone());
                    // Required to play videos in a Windows OS environment
                    // https://developer.apple.com/documentation/quicktime-file-format/handler_reference_atom/component_subtype
                    trak.mdia.hdlr.handler_type = FourCC::from_str("vide")?;
                }
                MediaBox::Hev1(hev1) => {
                    trak.mdia.minf.stbl.stsd.hev1 = Some(hev1.clone());
                    trak.mdia.hdlr.handler_type = FourCC::from_str("vide")?;
                }
                MediaBox::Vp9(vp09) => {
                    trak.mdia.minf.stbl.stsd.vp09 = Some(vp09.clone());
                    trak.mdia.hdlr.handler_type = FourCC::from_str("vide")?;
                }
                MediaBox::Mp4a(mp4a) => {
                    trak.mdia.minf.stbl.stsd.mp4a = Some(mp4a.clone());
                    trak.mdia.hdlr.handler_type = FourCC::from_str("soun")?;
                }
                MediaBox::Tx3g(tx3g) => {
                    trak.mdia.minf.stbl.stsd.tx3g = Some(tx3g.clone());
                    trak.mdia.hdlr.handler_type = FourCC::from_str("soun")?;
                }
                MediaBox::__Unknown => {}
            }

            let mut stts = SttsBox::default();
            stts.entries.append(&mut track.stts_entries);
            trak.mdia.minf.stbl.stts = stts;

            // Need
            trak.mdia.minf.stbl.stss = Some(StssBox::default());

            let mut stsc = StscBox::default();
            stsc.entries.append(&mut track.stsc_entries);
            trak.mdia.minf.stbl.stsc = stsc;

            let mut stsz = StszBox::default();
            stsz.sample_count = track.stsz_entries.len() as u32;
            stsz.sample_sizes.append(&mut track.stsz_entries);
            trak.mdia.minf.stbl.stsz = stsz;

            let mut co64 = Co64Box::default();
            co64.entries.append(&mut track.co64_entries);
            trak.mdia.minf.stbl.co64 = Some(co64);

            // stco and co64 never exist at the same time.
            trak.mdia.minf.stbl.stco = None;

            moov.traks.push(trak);

            // Calculate in milliseconds because mvhd timescale is different
            if moov_duration < duration / track.data.base.timescale as u64 {
                moov_duration = duration / track.data.base.timescale as u64;
            }

            moov.mvhd.next_track_id += 1;
        }

        moov.mvhd.duration = moov_duration * moov.mvhd.timescale as u64;
        moov.write_box(&mut self.writer)?;

        self.writer.seek(std::io::SeekFrom::Start(self.free_pos))?;
        BoxHeader::new(BoxType::MdatBox, mp4::HEADER_SIZE + self.free_size)
            .write(&mut self.writer)?;

        Ok(())
    }
}
