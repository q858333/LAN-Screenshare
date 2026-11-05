use anyhow::{Context, Result};
use openh264::decoder::Decoder;
use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::{YUVBuffer, YUVSource};
use openh264::OpenH264API;

pub struct H264Encoder {
    enc: Encoder,
    width: u32,
    height: u32,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32) -> Result<Self> {
        let config = EncoderConfig::new()
            .set_bitrate_bps(2_000_000)
            .max_frame_rate(fps as f32);
        let enc = Encoder::with_api_config(OpenH264API::from_source(), config)
            .context("openh264 encoder init")?;
        Ok(Self { enc, width, height })
    }

    pub fn encode_i420(&mut self, i420: &[u8]) -> Result<Vec<u8>> {
        let yuv = YUVBuffer::from_vec(
            i420.to_vec(),
            self.width as usize,
            self.height as usize,
        );
        let bitstream = self.enc.encode(&yuv).context("openh264 encode")?;
        Ok(bitstream.to_vec())
    }
}

pub struct H264Decoder {
    dec: Decoder,
}

impl Default for H264Decoder {
    fn default() -> Self {
        Self {
            dec: Decoder::new().expect("openh264 decoder"),
        }
    }
}

impl H264Decoder {
    pub fn decode_to_rgba(&mut self, annex_b: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>> {
        let Some(yuv) = self.dec.decode(annex_b)? else {
            return Ok(None);
        };
        let (w, h) = yuv.dimensions();
        let mut rgba = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some((w as u32, h as u32, rgba)))
    }
}
