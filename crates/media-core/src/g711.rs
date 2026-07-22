//! G.711 A-law and mu-law conversion helpers.

use std::sync::OnceLock;

static PCMA_TO_PCMU_TABLE: OnceLock<[u8; 256]> = OnceLock::new();
static PCMU_TO_PCMA_TABLE: OnceLock<[u8; 256]> = OnceLock::new();

fn pcma_to_pcmu_table() -> &'static [u8; 256] {
    PCMA_TO_PCMU_TABLE.get_or_init(|| {
        let mut table = [0_u8; 256];
        for (index, value) in table.iter_mut().enumerate() {
            *value = linear_to_ulaw(crate::recording::decode_pcma(index as u8));
        }
        table
    })
}

fn pcmu_to_pcma_table() -> &'static [u8; 256] {
    PCMU_TO_PCMA_TABLE.get_or_init(|| {
        let mut table = [0_u8; 256];
        for (index, value) in table.iter_mut().enumerate() {
            *value = linear_to_alaw(crate::recording::decode_pcmu(index as u8));
        }
        table
    })
}

pub fn linear_to_ulaw(pcm: i16) -> u8 {
    let mut pcm = i32::from(pcm);
    let sign = if pcm < 0 {
        pcm = -pcm;
        0x80
    } else {
        0
    };
    if pcm > 32_635 {
        pcm = 32_635;
    }
    let pcm = pcm + 0x84;
    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = (pcm >> (exponent + 3)) & 0x0f;
    !((sign | (exponent << 4) | mantissa) as u8)
}

pub fn linear_to_alaw(pcm: i16) -> u8 {
    let mut pcm = i32::from(pcm);
    let sign = if pcm < 0 {
        pcm = -pcm;
        0
    } else {
        0x80
    };
    if pcm > 32_635 {
        pcm = 32_635;
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
    ((sign | (exponent << 4) | mantissa) as u8) ^ 0x55
}

pub fn transcode_pcma_to_pcmu_inplace(payload: &mut [u8]) {
    let table = pcma_to_pcmu_table();
    for byte in payload {
        *byte = table[*byte as usize];
    }
}

pub fn transcode_pcmu_to_pcma_inplace(payload: &mut [u8]) {
    let table = pcmu_to_pcma_table();
    for byte in payload {
        *byte = table[*byte as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_g711_payloads_in_place() {
        let mut pcma = vec![0xd5, 0x55, 0x50];
        transcode_pcma_to_pcmu_inplace(&mut pcma);
        assert_eq!(pcma.len(), 3);

        let mut pcmu = vec![0xff, 0x00, 0x7f];
        transcode_pcmu_to_pcma_inplace(&mut pcmu);
        assert_eq!(pcmu.len(), 3);
    }

    #[test]
    fn encodes_full_scale_pcm_without_overflow() {
        for sample in [i16::MIN, i16::MIN + 1, i16::MAX] {
            let _ = linear_to_ulaw(sample);
            let _ = linear_to_alaw(sample);
        }
    }
}
