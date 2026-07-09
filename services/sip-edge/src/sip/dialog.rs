use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogValidationError {
    PeerMismatch,
    MissingFromTag,
    FromTagMismatch,
    ToTagMismatch,
    MissingCSeq,
    InvalidCSeq,
    CSeqOutOfOrder { received: u32, last: u32 },
}

impl DialogValidationError {
    pub(crate) fn status(&self) -> (u16, &'static str) {
        match self {
            Self::PeerMismatch
            | Self::MissingFromTag
            | Self::FromTagMismatch
            | Self::ToTagMismatch => (481, "Call/Transaction Does Not Exist"),
            Self::MissingCSeq | Self::InvalidCSeq => (400, "Bad Request"),
            Self::CSeqOutOfOrder { .. } => (500, "Server Internal Error"),
        }
    }
}

impl fmt::Display for DialogValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PeerMismatch => f.write_str("in-dialog request came from a different peer"),
            Self::MissingFromTag => f.write_str("missing From tag for in-dialog request"),
            Self::FromTagMismatch => f.write_str("in-dialog From tag does not match call dialog"),
            Self::ToTagMismatch => f.write_str("in-dialog To tag does not match call dialog"),
            Self::MissingCSeq => f.write_str("missing CSeq for in-dialog request"),
            Self::InvalidCSeq => f.write_str("invalid CSeq for in-dialog request"),
            Self::CSeqOutOfOrder { received, last } => write!(
                f,
                "out-of-order in-dialog CSeq: received {received}, last {last}"
            ),
        }
    }
}

pub(crate) fn tag_param(header_value: &str) -> Option<String> {
    semicolon_param(header_value, "tag")
}

pub(crate) fn cseq_number(cseq: &str) -> Option<u32> {
    cseq.split_whitespace().next()?.parse::<u32>().ok()
}

fn semicolon_param(header_value: &str, needle: &str) -> Option<String> {
    header_value.split(';').skip(1).find_map(|param| {
        let (name, value) = param.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case(needle)
            .then(|| value.trim().trim_matches('"').to_string())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::{cseq_number, tag_param};

    #[test]
    fn extracts_tag_parameter() {
        assert_eq!(
            tag_param("\"1001\" <sip:1001@example.com>;tag=from-tag;foo=bar"),
            Some("from-tag".to_string())
        );
        assert_eq!(
            tag_param("<sip:1001@example.com>;TAG=\"quoted-tag\""),
            Some("quoted-tag".to_string())
        );
        assert_eq!(tag_param("<sip:1001@example.com>"), None);
    }

    #[test]
    fn parses_cseq_number() {
        assert_eq!(cseq_number("12 INFO"), Some(12));
        assert_eq!(cseq_number("bad INFO"), None);
        assert_eq!(cseq_number(""), None);
    }
}
