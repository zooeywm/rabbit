use std::collections::HashMap;

use bytes::{BufMut as _, Bytes, BytesMut};
use eros::Context as _;
use reed_solomon_erasure::galois_8::ReedSolomon;

use crate::kernel::video_encoder::VideoFecPercentage;

const MAGIC: [u8; 4] = *b"RBF\x01";
const HEADER_SIZE: usize = 16;
const LENGTH_SIZE: usize = size_of::<u16>();
const MAX_DATA_SHARDS: usize = 32;

pub(crate) fn fec_rtp_packet_size(
    maximum_datagram_size: usize,
    percentage: VideoFecPercentage,
) -> eros::Result<usize> {
    let _ = percentage;
    Ok(maximum_datagram_size
        .checked_sub(HEADER_SIZE + LENGTH_SIZE)
        .with_context(|| {
            format!(
                "Video datagram size {maximum_datagram_size} cannot fit the {HEADER_SIZE}-byte FEC header"
            )
        })?)
}

pub(crate) fn encode_access_unit(
    packets: Vec<Bytes>,
    percentage: VideoFecPercentage,
) -> eros::Result<Vec<Bytes>> {
    let Some(first) = packets.first() else {
        return Ok(Vec::new());
    };
    let frame_id = rtp_timestamp(first)?;
    let block_count = packets.len().div_ceil(MAX_DATA_SHARDS);
    let block_count = u16::try_from(block_count).with_context(|| {
        format!(
            "Video access unit has too many FEC blocks for {} RTP packets",
            packets.len()
        )
    })?;
    let mut encoded = Vec::with_capacity(
        packets
            .len()
            .saturating_add(parity_shards(packets.len(), percentage)),
    );

    for (block_index, block) in packets.chunks(MAX_DATA_SHARDS).enumerate() {
        if block
            .iter()
            .any(|packet| rtp_timestamp(packet).ok() != Some(frame_id))
        {
            eros::bail!("Video access unit contains multiple RTP timestamps");
        }
        encode_block(
            &mut encoded,
            frame_id,
            u16::try_from(block_index).expect("FEC block index is bounded by block count"),
            block_count,
            block,
            percentage,
        )?;
    }

    Ok(encoded)
}

fn encode_block(
    encoded: &mut Vec<Bytes>,
    frame_id: u32,
    block_index: u16,
    block_count: u16,
    packets: &[Bytes],
    percentage: VideoFecPercentage,
) -> eros::Result<()> {
    let data_shards = packets.len();
    let parity_shards = parity_shards(data_shards, percentage);
    let shard_size = packets
        .iter()
        .map(Bytes::len)
        .max()
        .unwrap_or_default()
        .checked_add(LENGTH_SIZE)
        .with_context(|| "Video FEC shard size overflow")?;
    let mut shards = Vec::with_capacity(data_shards + parity_shards);
    for packet in packets {
        let packet_length = u16::try_from(packet.len()).with_context(|| {
            format!(
                "RTP packet is too large for FEC length metadata: {} bytes",
                packet.len()
            )
        })?;
        let mut shard = vec![0_u8; shard_size];
        shard[..LENGTH_SIZE].copy_from_slice(&packet_length.to_be_bytes());
        shard[LENGTH_SIZE..LENGTH_SIZE + packet.len()].copy_from_slice(packet);
        shards.push(shard);
    }
    shards.extend((0..parity_shards).map(|_| vec![0_u8; shard_size]));
    ReedSolomon::new(data_shards, parity_shards)
        .with_context(|| "Failed to configure video Reed-Solomon FEC")?
        .encode(&mut shards)
        .with_context(|| "Failed to encode video Reed-Solomon FEC")?;

    let data_shards =
        u8::try_from(data_shards).expect("FEC data shard count is bounded by MAX_DATA_SHARDS");
    let parity_shards =
        u8::try_from(parity_shards).expect("FEC parity shard count is bounded by percentage");
    for (shard_index, shard) in shards.into_iter().enumerate() {
        let mut payload = BytesMut::with_capacity(HEADER_SIZE + shard.len());
        payload.extend_from_slice(&MAGIC);
        payload.put_u32(frame_id);
        payload.put_u16(block_index);
        payload.put_u16(block_count);
        payload.put_u8(data_shards);
        payload.put_u8(parity_shards);
        payload.put_u8(
            u8::try_from(shard_index).expect("FEC shard index is bounded by data and parity"),
        );
        payload.put_u8(0);
        payload.extend_from_slice(&shard);
        encoded.push(payload.freeze());
    }
    Ok(())
}

fn parity_shards(data_shards: usize, percentage: VideoFecPercentage) -> usize {
    data_shards
        .saturating_mul(usize::from(percentage.get()))
        .div_ceil(100)
        .max(1)
}

fn rtp_timestamp(packet: &Bytes) -> eros::Result<u32> {
    let timestamp = packet
        .get(4..8)
        .with_context(|| format!("RTP packet is only {} bytes", packet.len()))?;
    Ok(u32::from_be_bytes(
        timestamp
            .try_into()
            .expect("RTP timestamp slice has exactly four bytes"),
    ))
}

#[derive(Default)]
pub(super) struct FecVideoReceiver {
    frame: Option<FecFrame>,
    last_completed_frame: Option<u32>,
}

struct FecFrame {
    frame_id: u32,
    block_count: u16,
    blocks: HashMap<u16, FecBlock>,
}

struct FecBlock {
    data_shards: usize,
    parity_shards: usize,
    shards: Vec<Option<Vec<u8>>>,
    packets: Option<Vec<Bytes>>,
}

impl FecVideoReceiver {
    pub(super) fn receive(&mut self, payload: Bytes) -> eros::Result<Option<Vec<Bytes>>> {
        if !payload.starts_with(&MAGIC) {
            return Ok(Some(vec![payload]));
        }
        let header = FecHeader::decode(&payload)?;
        if self.last_completed_frame.is_some_and(|completed| {
            completed == header.frame_id || (header.frame_id.wrapping_sub(completed) as i32) <= 0
        }) {
            return Ok(None);
        }
        if self
            .frame
            .as_ref()
            .is_some_and(|frame| frame.frame_id != header.frame_id)
        {
            let current = self
                .frame
                .as_ref()
                .expect("FEC frame was checked above")
                .frame_id;
            if (header.frame_id.wrapping_sub(current) as i32) <= 0 {
                return Ok(None);
            }
            self.frame = None;
        }
        let frame = self.frame.get_or_insert_with(|| FecFrame {
            frame_id: header.frame_id,
            block_count: header.block_count,
            blocks: HashMap::new(),
        });
        if frame.block_count != header.block_count {
            eros::bail!("Video FEC frame changed its block count");
        }
        let block = frame
            .blocks
            .entry(header.block_index)
            .or_insert_with(|| FecBlock {
                data_shards: header.data_shards,
                parity_shards: header.parity_shards,
                shards: vec![None; header.data_shards + header.parity_shards],
                packets: None,
            });
        if block.data_shards != header.data_shards || block.parity_shards != header.parity_shards {
            eros::bail!("Video FEC block changed its shard counts");
        }
        let shard = payload.slice(HEADER_SIZE..).to_vec();
        if let Some(existing) = &block.shards[header.shard_index] {
            if existing != &shard {
                eros::bail!("Video FEC shard was received twice with different data");
            }
        } else {
            block.shards[header.shard_index] = Some(shard);
        }
        block.try_reconstruct()?;

        if frame.blocks.len() != usize::from(frame.block_count)
            || frame.blocks.values().any(|block| block.packets.is_none())
        {
            return Ok(None);
        }
        let mut packets = Vec::new();
        for block_index in 0..frame.block_count {
            let block = frame
                .blocks
                .get_mut(&block_index)
                .with_context(|| format!("Video FEC block {block_index} is missing"))?;
            packets.extend(
                block
                    .packets
                    .take()
                    .with_context(|| format!("Video FEC block {block_index} is incomplete"))?,
            );
        }
        self.frame = None;
        self.last_completed_frame = Some(header.frame_id);
        Ok(Some(packets))
    }
}

impl FecBlock {
    fn try_reconstruct(&mut self) -> eros::Result<()> {
        if self.packets.is_some()
            || self.shards.iter().filter(|shard| shard.is_some()).count() < self.data_shards
        {
            return Ok(());
        }
        ReedSolomon::new(self.data_shards, self.parity_shards)
            .with_context(|| "Failed to configure video Reed-Solomon recovery")?
            .reconstruct(&mut self.shards)
            .with_context(|| "Failed to recover video Reed-Solomon shards")?;
        let mut packets = Vec::with_capacity(self.data_shards);
        for shard in self.shards.iter().take(self.data_shards) {
            let shard = shard
                .as_ref()
                .with_context(|| "Recovered video FEC data shard is missing")?;
            let length = shard
                .get(..LENGTH_SIZE)
                .with_context(|| "Recovered video FEC shard has no packet length")?;
            let length = usize::from(u16::from_be_bytes(
                length
                    .try_into()
                    .expect("FEC packet length slice has exactly two bytes"),
            ));
            let packet = shard
                .get(LENGTH_SIZE..LENGTH_SIZE + length)
                .with_context(|| "Recovered video FEC packet length exceeds its shard")?;
            packets.push(Bytes::copy_from_slice(packet));
        }
        self.packets = Some(packets);
        Ok(())
    }
}

struct FecHeader {
    frame_id: u32,
    block_index: u16,
    block_count: u16,
    data_shards: usize,
    parity_shards: usize,
    shard_index: usize,
}

impl FecHeader {
    fn decode(payload: &Bytes) -> eros::Result<Self> {
        if payload.len() <= HEADER_SIZE {
            eros::bail!("Video FEC datagram is too short: {} bytes", payload.len());
        }
        let frame_id = u32::from_be_bytes(payload[4..8].try_into().expect("fixed header field"));
        let block_index =
            u16::from_be_bytes(payload[8..10].try_into().expect("fixed header field"));
        let block_count =
            u16::from_be_bytes(payload[10..12].try_into().expect("fixed header field"));
        let data_shards = usize::from(payload[12]);
        let parity_shards = usize::from(payload[13]);
        let shard_index = usize::from(payload[14]);
        if block_count == 0 || block_index >= block_count {
            eros::bail!("Video FEC block index is outside its frame");
        }
        if data_shards == 0
            || data_shards > MAX_DATA_SHARDS
            || parity_shards == 0
            || shard_index >= data_shards + parity_shards
        {
            eros::bail!("Video FEC shard metadata is invalid");
        }
        Ok(Self {
            frame_id,
            block_index,
            block_count,
            data_shards,
            parity_shards,
            shard_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::{FecVideoReceiver, encode_access_unit, fec_rtp_packet_size};
    use crate::kernel::video_encoder::VideoFecPercentage;

    fn rtp_packet(sequence: u16, timestamp: u32, size: usize) -> Bytes {
        let mut packet = vec![0_u8; size.max(12)];
        packet[0] = 2 << 6;
        packet[2..4].copy_from_slice(&sequence.to_be_bytes());
        packet[4..8].copy_from_slice(&timestamp.to_be_bytes());
        Bytes::from(packet)
    }

    #[test]
    fn fec_recovers_dropped_video_datagrams() {
        let percentage = VideoFecPercentage::new(15).expect("valid FEC percentage");
        let original = (0..100)
            .map(|index| rtp_packet(index, 90_000, 100 + usize::from(index % 17)))
            .collect::<Vec<_>>();
        let encoded =
            encode_access_unit(original.clone(), percentage).expect("FEC encoding should succeed");
        let mut receiver = FecVideoReceiver::default();
        let mut recovered = None;
        for (index, packet) in encoded.into_iter().enumerate() {
            if index % 100 == 0 {
                continue;
            }
            recovered = receiver
                .receive(packet)
                .expect("FEC packet should be accepted")
                .or(recovered);
        }
        assert_eq!(recovered, Some(original));
    }

    #[test]
    fn fec_reserves_header_and_packet_length_in_each_datagram() {
        let percentage = VideoFecPercentage::DEFAULT;
        assert_eq!(
            fec_rtp_packet_size(1_200, percentage).expect("datagram should fit FEC"),
            1_182
        );
    }

    #[test]
    fn fec_does_not_publish_an_access_unit_beyond_its_recovery_budget() {
        let percentage = VideoFecPercentage::new(10).expect("valid FEC percentage");
        let original = (0..20)
            .map(|index| rtp_packet(index, 180_000, 200))
            .collect::<Vec<_>>();
        let encoded =
            encode_access_unit(original, percentage).expect("FEC encoding should succeed");
        let mut receiver = FecVideoReceiver::default();
        let mut published = None;
        for (index, packet) in encoded.into_iter().enumerate() {
            if index < 3 {
                continue;
            }
            published = receiver
                .receive(packet)
                .expect("FEC packet should be accepted")
                .or(published);
        }
        assert!(published.is_none());
    }
}
