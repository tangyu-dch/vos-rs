use tokio::io::{AsyncRead, AsyncReadExt};

use super::BoxError;

const MAX_SIP_MESSAGE_BYTES: usize = 1024 * 1024;

pub(super) struct SipFrameReader<R> {
    reader: R,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> SipFrameReader<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::with_capacity(4096),
        }
    }

    pub(super) async fn read_frame(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        loop {
            self.discard_keepalive_prefix();
            if let Some(frame_length) = complete_frame_length(&self.buffer)? {
                return Ok(Some(self.buffer.drain(..frame_length).collect()));
            }
            if self.buffer.len() >= MAX_SIP_MESSAGE_BYTES {
                return Err("SIP TCP 消息超过 1 MiB 限制".into());
            }
            let read = self.reader.read_buf(&mut self.buffer).await?;
            if read == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Err("SIP TCP 连接在完整消息前关闭".into());
            }
        }
    }

    fn discard_keepalive_prefix(&mut self) {
        while self.buffer.starts_with(b"\r\n") {
            self.buffer.drain(..2);
        }
    }
}

fn complete_frame_length(buffer: &[u8]) -> Result<Option<usize>, BoxError> {
    let Some((header_end, delimiter_length)) = find_header_end(buffer) else {
        return Ok(None);
    };
    let headers = std::str::from_utf8(&buffer[..header_end])?;
    let content_length = headers
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim().eq_ignore_ascii_case("content-length")
                || name.trim().eq_ignore_ascii_case("l"))
            .then(|| value.trim())
        })
        .map(str::parse::<usize>)
        .transpose()?
        .unwrap_or(0);
    let frame_length = header_end
        .checked_add(delimiter_length)
        .and_then(|length| length.checked_add(content_length))
        .ok_or("SIP TCP 消息长度溢出")?;
    if frame_length > MAX_SIP_MESSAGE_BYTES {
        return Err("SIP TCP 消息超过 1 MiB 限制".into());
    }
    Ok((buffer.len() >= frame_length).then_some(frame_length))
}

fn find_header_end(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })
}
