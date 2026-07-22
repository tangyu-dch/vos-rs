use async_nats::jetstream::{self, stream};
use async_nats::{header::NATS_MESSAGE_ID, HeaderMap};
use call_core::CallCdr;
use cdr_core::CdrEvent;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Debug)]
pub struct NatsCdrPublisher {
    jetstream: jetstream::Context,
    subject: String,
}

impl NatsCdrPublisher {
    pub async fn connect(
        nats_url: &str,
        subject: impl Into<String>,
        stream_name: impl Into<String>,
    ) -> Result<Self, AnyError> {
        let subject = subject.into();
        let stream_name = stream_name.into();
        let client = async_nats::connect(nats_url).await?;
        let jetstream = jetstream::new(client);
        jetstream
            .get_or_create_stream(stream::Config {
                name: stream_name,
                subjects: vec![subject.clone()],
                retention: stream::RetentionPolicy::WorkQueue,
                ..Default::default()
            })
            .await?;

        Ok(Self { jetstream, subject })
    }

    pub async fn publish_cdr(&self, cdr: &CallCdr) -> Result<(), AnyError> {
        let mut headers = HeaderMap::new();
        headers.insert(NATS_MESSAGE_ID, cdr.call_id.as_str());
        self.jetstream
            .publish_with_headers(
                self.subject.clone(),
                headers,
                CdrEvent::from_call_cdr(cdr).to_json_bytes().into(),
            )
            .await?
            .await?;
        Ok(())
    }
}
