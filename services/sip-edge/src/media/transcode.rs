//! G.711 PCMA (A-law) and PCMU (u-law) transcoding conversions.

pub fn linear_to_ulaw(mut pcm: i16) -> u8 {
    let sign = if pcm < 0 {
        pcm = -pcm;
        0x80
    } else {
        0
    };
    if pcm > 32635 {
        pcm = 32635;
    }
    let pcm = pcm + 0x84;
    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = (pcm >> (exponent + 3)) & 0x0f;
    let ulaw = (sign | (exponent << 4) | mantissa) as u8;
    !ulaw
}

pub fn linear_to_alaw(mut pcm: i16) -> u8 {
    let sign = if pcm < 0 {
        pcm = -pcm;
        0
    } else {
        0x80
    };
    if pcm > 32635 {
        pcm = 32635;
    }
    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = if exponent == 0 {
        (pcm >> 4) & 0x0f
    } else {
        (pcm >> (exponent + 3)) & 0x0f
    };
    let alaw = (sign | (exponent << 4) | mantissa) as u8;
    alaw ^ 0x55
}

pub fn transcode_pcma_to_pcmu(payload: &[u8]) -> Vec<u8> {
    payload
        .iter()
        .map(|&a| {
            let pcm = crate::media::recording::decode_pcma(a);
            linear_to_ulaw(pcm)
        })
        .collect()
}

pub fn transcode_pcmu_to_pcma(payload: &[u8]) -> Vec<u8> {
    payload
        .iter()
        .map(|&u| {
            let pcm = crate::media::recording::decode_pcmu(u);
            linear_to_alaw(pcm)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g711_roundtrip() {
        // Test a few sample values to verify roundtrip mapping behaves correctly
        for sample in [0, 100, -100, 1000, -1000, 5000, -5000] {
            let u = linear_to_ulaw(sample);
            let decoded_u = crate::media::recording::decode_pcmu(u);
            // G.711 compression is lossy, so check proximity
            assert!(
                (decoded_u - sample).abs() < 250,
                "pcm: {}, decoded: {}",
                sample,
                decoded_u
            );

            let a = linear_to_alaw(sample);
            let decoded_a = crate::media::recording::decode_pcma(a);
            assert!(
                (decoded_a - sample).abs() < 250,
                "pcm: {}, decoded: {}",
                sample,
                decoded_a
            );
        }
    }

    #[test]
    fn test_transcode_payloads() {
        let pcma = vec![0xd5, 0x55, 0x50];
        let pcmu = transcode_pcma_to_pcmu(&pcma);
        assert_eq!(pcmu.len(), 3);

        let pcmu_back = vec![0xff, 0x00, 0x7f];
        let pcma_back = transcode_pcmu_to_pcma(&pcmu_back);
        assert_eq!(pcma_back.len(), 3);
    }
}
